// SPDX-License-Identifier: GPL-3.0-or-later

//! Renders the live frames: once whenever something changes, and continuously
//! (about 30 times a second) while an effect is animating. Idle otherwise.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;

use crate::editor::{Editor, Topic};
use crate::{effects, screen_fx};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

struct Engine {
    editor: Weak<Editor>,
    cache: RefCell<screen_fx::Cache>,
    ticking: Cell<bool>,
}

pub fn start(editor: &Rc<Editor>) {
    let engine = Rc::new(Engine { editor: Rc::downgrade(editor), cache: RefCell::default(), ticking: Cell::new(false) });
    for topic in [Topic::Layout, Topic::Lighting, Topic::Screen, Topic::Effects, Topic::ScreenFx, Topic::KeyPress, Topic::Volume, Topic::Locks] {
        editor.connect(topic, glib::clone!(#[strong] engine, move || engine.wake()));
    }
    engine.wake();
}

impl Engine {
    fn wake(self: &Rc<Self>) {
        self.render();
        if self.ticking.get() || !self.animated() {
            return;
        }
        self.ticking.set(true);
        let engine = self.clone();
        glib::timeout_add_local(FRAME_INTERVAL, move || {
            engine.render();
            if engine.animated() {
                glib::ControlFlow::Continue
            } else {
                engine.ticking.set(false);
                glib::ControlFlow::Break
            }
        });
    }

    fn animated(&self) -> bool {
        let Some(editor) = self.editor.upgrade() else { return false };
        let (now, last) = (editor.now(), editor.last_press_time());
        let (lighting, screen) = (editor.lighting_fx(), editor.screen_fx());
        let volume_up = editor.volume().is_some_and(|v| v.showing(now))
            && (screen.volume || lighting.volume_bar != crate::effects::VolumeBar::Off);
        volume_up || lighting.is_animated(now, last) || screen.is_animated(now, last, &editor.animation())
    }

    fn render(&self) {
        let Some(editor) = self.editor.upgrade() else { return };
        let t = editor.now();
        let presses = editor.presses();
        let volume = editor.volume();
        let lighting =
            effects::render(&editor.lighting_fx(), &editor.output_keys(), &editor.lighting(), t, &presses, volume, editor.locks());
        let animation = editor.animation();
        let screen = {
            let image = editor.screen();
            let times = editor.press_times();
            let input = screen_fx::Inputs { image: &image, animation: &animation, t, press_times: &times, total_presses: editor.total_presses(), volume };
            screen_fx::render(&editor.screen_fx(), &input, &mut self.cache.borrow_mut())
        };
        editor.set_frames(lighting, screen);
    }
}
