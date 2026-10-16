// SPDX-License-Identifier: GPL-3.0-or-later

//! Talking to the keyboard: finding it, checking it is exactly the device the protocol was
//! verified against, and sending the live frames from a background thread. While a typing
//! effect is on, also reads the keyboard's own key reports.

pub mod apex5;
mod hidraw;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gtk::glib;

use crate::editor::{Editor, Topic};
use crate::effects::Typing;
use crate::screen_fx::ScreenTyping;
use hidraw::{Handle, Interface};

const SYSFS: &str = "/sys/class/hidraw";
const DEV: &str = "/dev";
/// At most about 30 frames per second go to the keyboard.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// How often to look for the keyboard while it isn't connected.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// The current frame is re-sent this often, in case the keyboard dropped it (after sleep, say).
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Off,
    Connected,
    NotFound,
    NoPermission,
    /// Found, but its HID descriptors differ from the verified ones, so nothing is sent.
    Unexpected,
    Failed(String),
}

type Packet = Box<[u8; apex5::FEATURE_LEN]>;
type StatusListener = Rc<dyn Fn(&Status)>;
/// An open keyboard interface and the parser for its key reports.
type KeySource = (Handle, fn(&[u8]) -> Vec<u8>);

/// Work for the writer thread. Newer frames replace older ones that weren't sent yet.
#[derive(Default)]
struct Pending {
    lighting: Option<Packet>,
    screen: Option<Packet>,
    reset: bool,
    stop: bool,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.lighting.is_none() && self.screen.is_none() && !self.reset
    }
}

#[derive(Default)]
struct Mailbox {
    pending: Mutex<Pending>,
    wake: Condvar,
}

impl Mailbox {
    fn update(&self, f: impl FnOnce(&mut Pending)) {
        f(&mut self.pending.lock().unwrap());
        self.wake.notify_one();
    }
}

enum Event {
    Failed(io::Error),
    Key(u8),
}

struct KeyReader {
    stop: Arc<AtomicBool>,
}

impl Drop for KeyReader {
    fn drop(&mut self) {
        // The thread notices within a quarter second and closes its files.
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Connection {
    id: u64,
    mailbox: Arc<Mailbox>,
    events: async_channel::Sender<Event>,
    interfaces: Vec<Interface>,
    keys: Option<KeyReader>,
    /// Last packets handed to the writer, so unchanged ones aren't sent again.
    sent: (Option<Packet>, Option<Packet>),
}

impl Connection {
    fn queue(&mut self, lighting: Packet, screen: Packet, force: bool) {
        let lighting = (force || self.sent.0.as_ref() != Some(&lighting)).then_some(lighting);
        let screen = (force || self.sent.1.as_ref() != Some(&screen)).then_some(screen);
        if lighting.is_none() && screen.is_none() {
            return;
        }
        self.mailbox.update(|p| {
            if let Some(packet) = &lighting {
                p.lighting = Some(packet.clone());
            }
            if let Some(packet) = &screen {
                p.screen = Some(packet.clone());
            }
        });
        if lighting.is_some() {
            self.sent.0 = lighting;
        }
        if screen.is_some() {
            self.sent.1 = screen;
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // The writer finishes what is queued (a lighting reset, say), then exits.
        self.mailbox.update(|p| p.stop = true);
    }
}

/// Keeps the keyboard in sync with the editor's live frames while enabled.
pub struct Link {
    editor: Rc<Editor>,
    enabled: Cell<bool>,
    status: RefCell<Status>,
    connection: RefCell<Option<Connection>>,
    next_id: Cell<u64>,
    listeners: RefCell<Vec<StatusListener>>,
}

impl Link {
    pub fn new(editor: &Rc<Editor>) -> Rc<Self> {
        let link = Rc::new(Self {
            editor: editor.clone(),
            enabled: Cell::new(false),
            status: RefCell::new(Status::Off),
            connection: RefCell::default(),
            next_id: Cell::new(0),
            listeners: RefCell::default(),
        });
        let weak = Rc::downgrade(&link);
        editor.connect(Topic::Frame, glib::clone!(#[strong] weak, move || {
            if let Some(link) = weak.upgrade() {
                link.push_frame(false);
            }
        }));
        for topic in [Topic::Effects, Topic::ScreenFx] {
            editor.connect(topic, glib::clone!(#[strong] weak, move || {
                if let Some(link) = weak.upgrade() {
                    link.update_key_reader();
                }
            }));
        }
        glib::timeout_add_local(RETRY_INTERVAL, glib::clone!(#[strong] weak, move || {
            let Some(link) = weak.upgrade() else { return glib::ControlFlow::Break };
            if link.enabled.get() && link.connection.borrow().is_none() {
                link.connect();
            }
            glib::ControlFlow::Continue
        }));
        glib::timeout_add_local(REFRESH_INTERVAL, move || {
            let Some(link) = weak.upgrade() else { return glib::ControlFlow::Break };
            link.push_frame(true);
            glib::ControlFlow::Continue
        });
        link
    }

    pub fn connect_status(&self, f: impl Fn(&Status) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    fn set_status(&self, status: Status) {
        if *self.status.borrow() == status {
            return;
        }
        *self.status.borrow_mut() = status.clone();
        let listeners: Vec<_> = self.listeners.borrow().clone();
        for f in listeners {
            f(&status);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.get()
    }

    pub fn set_enabled(self: &Rc<Self>, on: bool) {
        if self.enabled.replace(on) == on {
            return;
        }
        if on {
            self.connect();
        } else {
            self.disconnect();
            self.set_status(Status::Off);
        }
    }

    /// Hands the lighting back to the keyboard's own lighting and stops sending.
    pub fn restore_keyboard_lighting(self: &Rc<Self>) -> Result<(), String> {
        if let Some(connection) = self.connection.borrow().as_ref() {
            connection.mailbox.update(|p| {
                p.lighting = None;
                p.screen = None;
                p.reset = true;
            });
        } else {
            send_lighting_reset()?;
        }
        self.set_enabled(false);
        Ok(())
    }

    fn connect(self: &Rc<Self>) {
        if self.connection.borrow().is_some() {
            return;
        }
        let (handle, interfaces) = match open_control() {
            Ok(found) => found,
            Err(status) => return self.set_status(status),
        };
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        let mailbox = Arc::new(Mailbox::default());
        let (events, received) = async_channel::unbounded();
        let spawned = std::thread::Builder::new().name("keyboard output".into()).spawn(glib::clone!(
            #[strong] mailbox,
            #[strong] events,
            move || write_frames(handle, &mailbox, &events)
        ));
        if let Err(e) = spawned {
            return self.set_status(Status::Failed(e.to_string()));
        }

        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = received.recv().await {
                let Some(link) = weak.upgrade() else { break };
                match event {
                    Event::Key(led) => link.editor.key_pressed(led),
                    Event::Failed(error) => link.lost(id, error),
                }
            }
        });

        *self.connection.borrow_mut() =
            Some(Connection { id, mailbox, events, interfaces, keys: None, sent: (None, None) });
        self.set_status(Status::Connected);
        self.push_frame(true);
        self.update_key_reader();
    }

    fn disconnect(&self) {
        self.connection.take();
        self.editor.set_hardware_typing(false);
    }

    /// The writer failed; the retry timer reconnects once the keyboard is back.
    fn lost(&self, id: u64, error: io::Error) {
        if self.connection.borrow().as_ref().is_none_or(|c| c.id != id) {
            return; // a connection that was already replaced
        }
        self.disconnect();
        self.set_status(match error.raw_os_error() {
            Some(libc::ENODEV) | Some(libc::ENOENT) => Status::NotFound,
            _ => Status::Failed(error.to_string()),
        });
    }

    fn push_frame(&self, force: bool) {
        let mut connection = self.connection.borrow_mut();
        let Some(connection) = connection.as_mut() else { return };
        let lighting = {
            let live = self.editor.live_lighting();
            let colors: Vec<_> = self.editor.output_keys().iter().map(|k| (k.led, live.get(k.led))).collect();
            Box::new(apex5::lighting_packet(&colors))
        };
        let screen = Box::new(apex5::screen_packet(&self.editor.live_screen()));
        connection.queue(lighting, screen, force);
    }

    /// Reads the keyboard's key reports only while a typing effect needs them.
    fn update_key_reader(&self) {
        let wanted = self.editor.lighting_fx().typing != Typing::Off || self.editor.screen_fx().typing != ScreenTyping::Off;
        let mut connection = self.connection.borrow_mut();
        let reading = match connection.as_mut() {
            Some(c) => {
                if wanted && c.keys.is_none() {
                    c.keys = start_key_reader(&c.interfaces, c.events.clone());
                } else if !wanted {
                    c.keys = None;
                }
                c.keys.is_some()
            }
            None => false,
        };
        self.editor.set_hardware_typing(reading);
    }
}

/// Where to look for hidraw devices. Debug builds can be pointed elsewhere for testing
/// (`OPENSTEEL_SYSFS`), so a test run never reaches a real keyboard.
fn sysfs() -> std::path::PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("OPENSTEEL_SYSFS") {
        return path.into();
    }
    SYSFS.into()
}

/// Finds the keyboard and opens its control interface, after checking its descriptor.
fn open_control() -> Result<(Handle, Vec<Interface>), Status> {
    let found = hidraw::find(&sysfs(), Path::new(DEV), apex5::VENDOR, apex5::PRODUCT);
    // With several Apex 5s connected, use the first one.
    let Some(port) = found.first().map(|i| i.port.clone()) else { return Err(Status::NotFound) };
    let interfaces: Vec<Interface> = found.into_iter().filter(|i| i.port == port).collect();
    let control = interfaces.iter().find(|i| i.number == apex5::CONTROL_INTERFACE).ok_or(Status::NotFound)?;
    if control.descriptor != apex5::CONTROL_DESCRIPTOR {
        return Err(Status::Unexpected);
    }
    let handle = Handle::open(&control.node, true).map_err(|e| match e.kind() {
        io::ErrorKind::PermissionDenied => Status::NoPermission,
        io::ErrorKind::NotFound => Status::NotFound,
        _ => Status::Failed(e.to_string()),
    })?;
    Ok((handle, interfaces))
}

/// The only way packets leave this module: a final check that each is one of the three
/// known commands, in the exact size the descriptor declares.
fn send(handle: &mut Handle, packet: &[u8]) -> io::Result<()> {
    match (packet.len(), packet.first(), packet.get(1)) {
        (apex5::FEATURE_LEN, Some(0), Some(&apex5::CMD_LIGHTING | &apex5::CMD_SCREEN)) => handle.send_feature(packet),
        (apex5::OUTPUT_LEN, Some(0), Some(&apex5::CMD_LIGHTING_RESET)) => handle.send_output(packet),
        _ => Err(io::Error::other("refused to send an unknown packet")),
    }
}

fn write_frames(mut handle: Handle, mailbox: &Mailbox, events: &async_channel::Sender<Event>) {
    let mut last_write = Instant::now() - MIN_FRAME_INTERVAL;
    loop {
        {
            let mut pending = mailbox.pending.lock().unwrap();
            while pending.is_empty() && !pending.stop {
                pending = mailbox.wake.wait(pending).unwrap();
            }
            if pending.is_empty() {
                return; // stopped, with nothing left to send
            }
        }
        // Keep to the frame rate; frames that arrive meanwhile replace the waiting ones.
        std::thread::sleep((last_write + MIN_FRAME_INTERVAL).saturating_duration_since(Instant::now()));
        let (lighting, screen, reset) = {
            let mut pending = mailbox.pending.lock().unwrap();
            (pending.lighting.take(), pending.screen.take(), std::mem::take(&mut pending.reset))
        };
        last_write = Instant::now();
        let result = lighting
            .iter()
            .chain(&screen)
            .try_for_each(|packet| send(&mut handle, &packet[..]))
            .and_then(|()| if reset { send(&mut handle, &apex5::lighting_reset_packet()) } else { Ok(()) });
        if let Err(error) = result {
            let _ = events.send_blocking(Event::Failed(error));
            return;
        }
    }
}

fn start_key_reader(interfaces: &[Interface], events: async_channel::Sender<Event>) -> Option<KeyReader> {
    let open = |number: u8, descriptor: &[u8]| {
        let interface = interfaces.iter().find(|i| i.number == number && i.descriptor == descriptor)?;
        Handle::open(&interface.node, false).ok()
    };
    let mut sources: Vec<KeySource> = Vec::new();
    if let Some(handle) = open(apex5::BOOT_KEYS_INTERFACE, apex5::BOOT_KEYS_DESCRIPTOR) {
        sources.push((handle, apex5::boot_keys));
    }
    if let Some(handle) = open(apex5::ALL_KEYS_INTERFACE, apex5::ALL_KEYS_DESCRIPTOR) {
        sources.push((handle, apex5::all_keys));
    }
    if sources.is_empty() {
        return None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    std::thread::Builder::new()
        .name("keyboard keys".into())
        .spawn(glib::clone!(#[strong] stop, move || read_keys(sources, &stop, &events)))
        .ok()?;
    Some(KeyReader { stop })
}

/// Passes on each newly pressed key's code. Nothing is stored or logged here.
fn read_keys(mut sources: Vec<KeySource>, stop: &AtomicBool, events: &async_channel::Sender<Event>) {
    let mut held: Vec<HashSet<u8>> = vec![HashSet::new(); sources.len()];
    let mut held_before: HashSet<u8> = HashSet::new();
    let mut buf = [0u8; 64];
    while !stop.load(Ordering::Relaxed) {
        let ready = {
            let handles: Vec<&Handle> = sources.iter().map(|(h, _)| h).collect();
            match hidraw::poll(&handles, Duration::from_millis(250)) {
                Ok(ready) => ready,
                Err(_) => return,
            }
        };
        for (i, (handle, parse)) in sources.iter_mut().enumerate() {
            if ready[i] {
                let Ok(n) = handle.read(&mut buf) else { return };
                held[i] = parse(&buf[..n]).into_iter().collect();
            }
        }
        // A key counts once, even when both interfaces report it.
        let now: HashSet<u8> = held.iter().flatten().copied().collect();
        for &led in now.difference(&held_before) {
            if events.send_blocking(Event::Key(led)).is_err() {
                return;
            }
        }
        held_before = now;
    }
}

/// Hands the lighting back to the keyboard without a running connection.
pub fn send_lighting_reset() -> Result<(), String> {
    let (mut handle, _) = open_control().map_err(|status| describe(&status))?;
    send(&mut handle, &apex5::lighting_reset_packet()).map_err(|e| e.to_string())
}

pub fn describe(status: &Status) -> String {
    match status {
        Status::Off => "Not sending to the keyboard".into(),
        Status::Connected => "Connected to the Apex 5".into(),
        Status::NotFound => "No Apex 5 found".into(),
        Status::NoPermission => "No permission to use the keyboard (see the README)".into(),
        Status::Unexpected => "The keyboard's USB descriptors don't match the verified Apex 5, so nothing is sent".into(),
        Status::Failed(e) => format!("Couldn't talk to the keyboard: {e}"),
    }
}

/// A read-only check for the command line: finds the keyboard and verifies its descriptors.
pub fn probe() -> String {
    let found = hidraw::find(&sysfs(), Path::new(DEV), apex5::VENDOR, apex5::PRODUCT);
    if found.is_empty() {
        return "No SteelSeries Apex 5 (1038:161c) found.".into();
    }
    let mut report = String::from("SteelSeries Apex 5 (1038:161c)\n");
    for interface in &found {
        let expected = match interface.number {
            apex5::CONTROL_INTERFACE => Some(("lighting and screen", apex5::CONTROL_DESCRIPTOR)),
            apex5::BOOT_KEYS_INTERFACE => Some(("keys (boot)", apex5::BOOT_KEYS_DESCRIPTOR)),
            apex5::ALL_KEYS_INTERFACE => Some(("keys (all)", apex5::ALL_KEYS_DESCRIPTOR)),
            _ => None,
        };
        let (role, verdict) = match expected {
            Some((role, d)) if interface.descriptor == d => (role, "descriptor matches"),
            Some((role, _)) => (role, "DESCRIPTOR DIFFERS"),
            None => ("not used", "-"),
        };
        let access = match Handle::open(&interface.node, interface.number == apex5::CONTROL_INTERFACE) {
            Ok(_) => "accessible",
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => "no permission",
            Err(_) => "can't open",
        };
        report += &format!(
            "  interface {} {}: {role}, {verdict}, {access}\n",
            interface.number,
            interface.node.display()
        );
    }
    report += &format!("Status: {}", describe(&match open_control() {
        Ok(_) => Status::Connected,
        Err(status) => status,
    }).replace("Connected to", "Ready to use"));
    report
}
