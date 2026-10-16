// SPDX-License-Identifier: GPL-3.0-or-later

//! Follows the system volume through PipeWire / PulseAudio (`pactl`), so the keyboard can
//! show a slider whenever it changes. Only ever reads the volume, never sets it.

use std::cell::{Cell, RefCell};
use std::io::{self, BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;

use crate::editor::{Editor, Topic};
use crate::effects::VolumeBar;

/// Seconds the volume stays on the keyboard after a change.
pub const HOLD: f64 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    /// 1.0 is 100 %; PipeWire allows boosting up to 1.5.
    pub level: f64,
    pub muted: bool,
    /// When it last changed (editor seconds); `None` until the first change after startup.
    pub changed_at: Option<f64>,
}

impl Level {
    /// Whether the volume display should be up at `now`.
    pub fn showing(&self, now: f64) -> bool {
        self.changed_at.is_some_and(|at| (0.0..=HOLD).contains(&(now - at)))
    }
}

enum Event {
    /// The volume when watching started: remembered, but not shown.
    Initial(f64, bool),
    Changed(f64, bool),
}

/// The running `pactl subscribe`; stopping it ends the reader thread too.
struct Watcher {
    child: Child,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pactl() -> Command {
    let mut command = Command::new("pactl");
    // Untranslated output, so it can be parsed.
    command.env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null());
    command
}

fn query() -> Option<(f64, bool)> {
    let run = |args: &[&str]| {
        let output = pactl().args(args).output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let level = parse_level(&run(&["get-sink-volume", "@DEFAULT_SINK@"])?)?;
    let muted = parse_muted(&run(&["get-sink-mute", "@DEFAULT_SINK@"])?)?;
    Some((level, muted))
}

/// Reads the average channel volume from `pactl get-sink-volume` output.
pub fn parse_level(text: &str) -> Option<f64> {
    let first = text.lines().next()?.strip_prefix("Volume:")?;
    let percents: Vec<f64> = first
        .split('/')
        .filter_map(|part| part.trim().strip_suffix('%')?.trim().parse().ok())
        .collect();
    (!percents.is_empty()).then(|| percents.iter().sum::<f64>() / percents.len() as f64 / 100.0)
}

/// Reads `pactl get-sink-mute` output.
pub fn parse_muted(text: &str) -> Option<bool> {
    match text.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn watch(events: async_channel::Sender<Event>) -> io::Result<Watcher> {
    let mut command = pactl();
    command.arg("subscribe").stdout(Stdio::piped());
    // SAFETY: prctl is async-signal-safe. It ends pactl if the app dies without cleaning up.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| io::Error::other("no output from pactl"))?;
    std::thread::Builder::new().name("volume".into()).spawn(move || {
        let mut last = query();
        if let Some((level, muted)) = last
            && events.send_blocking(Event::Initial(level, muted)).is_err()
        {
            return;
        }
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            // Volume changes arrive as sink events; switching outputs as a server event.
            if !(line.contains(" on sink #") || line.contains(" on server")) {
                continue;
            }
            let now = query();
            if now != last
                && let Some((level, muted)) = now
                && events.send_blocking(Event::Changed(level, muted)).is_err()
            {
                break;
            }
            last = now;
        }
    })?;
    Ok(Watcher { child })
}

/// Runs the watcher while a volume display is turned on.
struct Follower {
    editor: Weak<Editor>,
    watcher: RefCell<Option<Watcher>>,
    /// Tells the current watcher apart from ones that were already replaced.
    generation: Cell<u64>,
}

impl Follower {
    fn update(self: &Rc<Self>) {
        let Some(editor) = self.editor.upgrade() else { return };
        let wanted = editor.screen_fx().volume || editor.lighting_fx().volume_bar != VolumeBar::Off;
        if !wanted {
            self.watcher.take();
            return;
        }
        if self.watcher.borrow().is_some() {
            return;
        }
        let (sender, events) = async_channel::unbounded();
        match watch(sender) {
            Ok(started) => *self.watcher.borrow_mut() = Some(started),
            Err(e) => return eprintln!("Can't follow the volume (is pactl installed?): {e}"),
        }
        let id = self.generation.get() + 1;
        self.generation.set(id);
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(editor) = this.upgrade().and_then(|f| f.editor.upgrade()) else { return };
                match event {
                    Event::Initial(level, muted) => editor.set_volume(level, muted, false),
                    Event::Changed(level, muted) => editor.set_volume(level, muted, true),
                }
            }
            // pactl stopped (the sound server restarted, say): try again shortly.
            let Some(follower) = this.upgrade() else { return };
            if follower.generation.get() == id {
                follower.watcher.take();
                glib::timeout_add_local_once(Duration::from_secs(5), move || {
                    if let Some(follower) = this.upgrade() {
                        follower.update();
                    }
                });
            }
        });
    }
}

pub fn follow(editor: &Rc<Editor>) {
    let follower =
        Rc::new(Follower { editor: Rc::downgrade(editor), watcher: RefCell::default(), generation: Cell::new(0) });
    follower.update();
    for topic in [Topic::Effects, Topic::ScreenFx] {
        editor.connect(topic, glib::clone!(#[strong] follower, move || follower.update()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_volume() {
        let stereo = "Volume: front-left: 34080 /  52% / -17.04 dB,   front-right: 34080 /  52% / -17.04 dB\n        balance 0.00\n";
        assert_eq!(parse_level(stereo), Some(0.52));
        let uneven = "Volume: front-left: 65536 / 100% / 0.00 dB,   front-right: 32768 /  50% / -18.06 dB\n";
        assert_eq!(parse_level(uneven), Some(0.75));
        assert_eq!(parse_level("Volume: mono: 98304 / 150% / 10.57 dB\n"), Some(1.5));
        assert_eq!(parse_level("Failure: No such entity\n"), None);
    }

    #[test]
    fn parses_mute() {
        assert_eq!(parse_muted("Mute: no\n"), Some(false));
        assert_eq!(parse_muted("Mute: yes\n"), Some(true));
        assert_eq!(parse_muted("Stumm: ja\n"), None);
    }

    /// Needs a running sound server, so run it by hand: `cargo test real_volume -- --ignored`.
    #[test]
    #[ignore]
    fn reads_the_real_volume() {
        let (level, muted) = query().expect("pactl should report the default output");
        println!("volume {:.0}%, muted: {muted}", level * 100.0);
        assert!((0.0..=1.6).contains(&level));
    }

    #[test]
    fn shows_for_a_moment_after_a_change() {
        let level = Level { level: 0.5, muted: false, changed_at: Some(10.0) };
        assert!(level.showing(10.0) && level.showing(12.0));
        assert!(!level.showing(10.0 + HOLD + 0.1));
        assert!(!Level { changed_at: None, ..level }.showing(10.0), "the startup value isn't shown");
    }
}
