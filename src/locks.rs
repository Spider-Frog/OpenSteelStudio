// SPDX-License-Identifier: GPL-3.0-or-later

//! Follows Caps Lock, Num Lock and Scroll Lock through the keyboard's lock lights, which
//! Linux shows to every user in /sys/class/leds. No key presses are involved.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;

use crate::device::apex5;
use crate::editor::{Editor, Topic};

const LEDS: &str = "/sys/class/leds";
const POLL: Duration = Duration::from_millis(250);
/// Look for the lock lights again every this many polls, in case the keyboard was replugged.
const RESCAN_EVERY: u32 = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Locks {
    pub caps: bool,
    pub num: bool,
    pub scroll: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lock {
    Caps,
    Num,
    Scroll,
}

/// The lock lights to read: the Apex 5's own, or every keyboard's when it isn't connected.
fn find(root: &Path) -> Vec<(Lock, PathBuf)> {
    let apex = format!("HID_ID=0003:{:08X}:{:08X}", apex5::VENDOR, apex5::PRODUCT);
    let Ok(entries) = std::fs::read_dir(root) else { return vec![] };
    let (mut own, mut others) = (Vec::new(), Vec::new());
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lock = match name.rsplit_once("::").map(|(_, kind)| kind) {
            Some("capslock") => Lock::Caps,
            Some("numlock") => Lock::Num,
            Some("scrolllock") => Lock::Scroll,
            _ => continue,
        };
        // The light belongs to an input device, which belongs to a HID device with an id.
        let uevent = std::fs::read_to_string(entry.path().join("device/device/uevent")).unwrap_or_default();
        let is_apex = uevent.lines().any(|line| line.eq_ignore_ascii_case(&apex));
        if is_apex { &mut own } else { &mut others }.push((lock, entry.path()));
    }
    if own.is_empty() { others } else { own }
}

/// A lock counts as on when any of its lights is on.
fn read(lights: &[(Lock, PathBuf)]) -> Locks {
    let mut locks = Locks::default();
    for (lock, path) in lights {
        let on = std::fs::read_to_string(path.join("brightness")).is_ok_and(|b| b.trim() != "0");
        match lock {
            Lock::Caps => locks.caps |= on,
            Lock::Num => locks.num |= on,
            Lock::Scroll => locks.scroll |= on,
        }
    }
    locks
}

struct Follower {
    editor: Weak<Editor>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Follower {
    /// Polls while lock key colors are on, and stops otherwise.
    fn update(self: &Rc<Self>) {
        let Some(editor) = self.editor.upgrade() else { return };
        let wanted = editor.lighting_fx().lock_keys.enabled;
        if !wanted {
            if let Some(timer) = self.timer.take() {
                timer.remove();
            }
            return;
        }
        if self.timer.borrow().is_some() {
            return;
        }
        let mut lights = find(Path::new(LEDS));
        let mut polls = 0;
        editor.set_locks(read(&lights));
        let this = Rc::downgrade(self);
        let timer = glib::timeout_add_local(POLL, move || {
            let Some(editor) = this.upgrade().and_then(|f| f.editor.upgrade()) else {
                return glib::ControlFlow::Break;
            };
            polls += 1;
            if polls % RESCAN_EVERY == 0 {
                lights = find(Path::new(LEDS));
            }
            editor.set_locks(read(&lights));
            glib::ControlFlow::Continue
        });
        *self.timer.borrow_mut() = Some(timer);
    }
}

pub fn follow(editor: &Rc<Editor>) {
    let follower = Rc::new(Follower { editor: Rc::downgrade(editor), timer: RefCell::default() });
    follower.update();
    editor.connect(Topic::Effects, glib::clone!(#[strong] follower, move || follower.update()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(root: &Path, name: &str, hid_id: &str, on: bool) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("device/device")).unwrap();
        std::fs::write(dir.join("brightness"), if on { "1\n" } else { "0\n" }).unwrap();
        std::fs::write(dir.join("device/device/uevent"), format!("DRIVER=hid-generic\nHID_ID={hid_id}\n")).unwrap();
    }

    #[test]
    fn prefers_the_apex_5() {
        let root = std::env::temp_dir().join(format!("open-steel-studio-leds-{}", std::process::id()));
        let apex = "0003:00001038:0000161C";
        let other = "0003:0000046D:00004082";
        light(&root, "input3::capslock", apex, true);
        light(&root, "input3::numlock", apex, false);
        light(&root, "input5::numlock", apex, true); // the second keyboard interface
        light(&root, "input42::scrolllock", other, true); // another keyboard: ignored
        light(&root, "input42::capslock", other, false);
        std::fs::create_dir_all(root.join("phy0-led")).unwrap(); // not a lock light
        let found = find(&root);
        let locks = read(&found);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(found.len(), 3);
        assert_eq!(locks, Locks { caps: true, num: true, scroll: false });
    }

    #[test]
    fn falls_back_to_any_keyboard() {
        let root = std::env::temp_dir().join(format!("open-steel-studio-leds-other-{}", std::process::id()));
        light(&root, "input8::capslock", "0003:00002516:0000017B", true);
        let locks = read(&find(&root));
        std::fs::remove_dir_all(&root).ok();
        assert!(locks.caps);
    }
}
