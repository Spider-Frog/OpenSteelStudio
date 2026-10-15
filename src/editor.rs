// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared editor state with simple change notification and undo.
//!
//! Pages subscribe to a [`Topic`] and redraw when it is emitted. Mutating
//! methods always release their borrows before emitting, so listeners are free
//! to read the state.

use std::cell::{Cell, Ref, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::layout::{self, Key, Region};
use crate::model::{Bitmap, Lighting, Rgb};

const UNDO_LIMIT: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Topic {
    Layout,
    Lighting,
    Selection,
    Color,
    Tool,
    Screen,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyTool {
    Select,
    Paint,
    Pick,
}

type Listener = (Topic, Rc<dyn Fn()>);

pub struct Editor {
    region: Cell<Region>,
    keys: RefCell<Rc<Vec<Key>>>,
    lighting: RefCell<Lighting>,
    selection: RefCell<BTreeSet<u8>>,
    color: Cell<Rgb>,
    tool: Cell<KeyTool>,
    tool_before_pick: Cell<KeyTool>,
    screen: RefCell<Bitmap>,
    lighting_undo: RefCell<Vec<Lighting>>,
    screen_undo: RefCell<Vec<Bitmap>>,
    listeners: RefCell<Vec<Listener>>,
}

impl Editor {
    pub fn new(region: Region, lighting: Lighting, screen: Bitmap) -> Rc<Self> {
        Rc::new(Self {
            region: Cell::new(region),
            keys: RefCell::new(Rc::new(layout::keys(region))),
            lighting: RefCell::new(lighting),
            selection: RefCell::default(),
            color: Cell::new(Rgb::new(0x00, 0xc8, 0xff)),
            tool: Cell::new(KeyTool::Select),
            tool_before_pick: Cell::new(KeyTool::Select),
            screen: RefCell::new(screen),
            lighting_undo: RefCell::default(),
            screen_undo: RefCell::default(),
            listeners: RefCell::default(),
        })
    }

    pub fn connect(&self, topic: Topic, f: impl Fn() + 'static) {
        self.listeners.borrow_mut().push((topic, Rc::new(f)));
    }

    fn emit(&self, topic: Topic) {
        let listeners: Vec<_> = self
            .listeners
            .borrow()
            .iter()
            .filter(|(t, _)| *t == topic)
            .map(|(_, f)| f.clone())
            .collect();
        for f in listeners {
            f();
        }
    }

    // Layout

    pub fn region(&self) -> Region {
        self.region.get()
    }

    pub fn set_region(&self, region: Region) {
        if self.region.replace(region) == region {
            return;
        }
        let keys = Rc::new(layout::keys(region));
        self.selection.borrow_mut().retain(|led| keys.iter().any(|k| k.led == *led));
        *self.keys.borrow_mut() = keys;
        self.emit(Topic::Layout);
        self.emit(Topic::Selection);
    }

    pub fn keys(&self) -> Rc<Vec<Key>> {
        self.keys.borrow().clone()
    }

    // Lighting

    pub fn lighting(&self) -> Ref<'_, Lighting> {
        self.lighting.borrow()
    }

    /// Applies `f` as one undoable step.
    pub fn edit_lighting(&self, f: impl FnOnce(&mut Lighting)) {
        let before = self.lighting.borrow().clone();
        f(&mut self.lighting.borrow_mut());
        if *self.lighting.borrow() != before {
            push_limited(&self.lighting_undo, before);
            self.emit(Topic::Lighting);
        }
    }

    /// Starts a paint stroke: one undo step covers every [`Self::paint_key`] until the next stroke.
    pub fn begin_lighting_stroke(&self) {
        push_limited(&self.lighting_undo, self.lighting.borrow().clone());
    }

    /// Drops the stroke's undo step again if the stroke changed nothing.
    pub fn end_lighting_stroke(&self) {
        drop_if_unchanged(&self.lighting_undo, &self.lighting.borrow());
    }

    pub fn paint_key(&self, led: u8, color: Rgb) {
        if self.lighting.borrow().get(led) == color {
            return;
        }
        self.lighting.borrow_mut().set(led, color);
        self.emit(Topic::Lighting);
    }

    pub fn undo_lighting(&self) -> bool {
        let Some(previous) = self.lighting_undo.borrow_mut().pop() else {
            return false;
        };
        *self.lighting.borrow_mut() = previous;
        self.emit(Topic::Lighting);
        true
    }

    // Selection

    pub fn selection(&self) -> Ref<'_, BTreeSet<u8>> {
        self.selection.borrow()
    }

    pub fn set_selection(&self, selection: BTreeSet<u8>) {
        if *self.selection.borrow() == selection {
            return;
        }
        *self.selection.borrow_mut() = selection;
        self.emit(Topic::Selection);
    }

    /// LEDs a lighting action should affect: the selection, or every key when nothing is selected.
    pub fn target_leds(&self) -> Vec<u8> {
        let selection = self.selection.borrow();
        if selection.is_empty() {
            self.keys().iter().map(|k| k.led).collect()
        } else {
            selection.iter().copied().collect()
        }
    }

    // Current color and tool

    pub fn color(&self) -> Rgb {
        self.color.get()
    }

    pub fn set_color(&self, color: Rgb) {
        if self.color.replace(color) != color {
            self.emit(Topic::Color);
        }
    }

    pub fn tool(&self) -> KeyTool {
        self.tool.get()
    }

    pub fn set_tool(&self, tool: KeyTool) {
        let previous = self.tool.replace(tool);
        if previous == tool {
            return;
        }
        if tool == KeyTool::Pick {
            self.tool_before_pick.set(previous);
        }
        self.emit(Topic::Tool);
    }

    /// Returns from the color picker to whichever tool was active before it.
    pub fn finish_pick(&self) {
        self.set_tool(self.tool_before_pick.get());
    }

    // Screen

    pub fn screen(&self) -> Ref<'_, Bitmap> {
        self.screen.borrow()
    }

    /// Applies `f` as one undoable step.
    pub fn edit_screen(&self, f: impl FnOnce(&mut Bitmap)) {
        let before = self.screen.borrow().clone();
        f(&mut self.screen.borrow_mut());
        if *self.screen.borrow() != before {
            push_limited(&self.screen_undo, before);
            self.emit(Topic::Screen);
        }
    }

    pub fn begin_screen_stroke(&self) {
        push_limited(&self.screen_undo, self.screen.borrow().clone());
    }

    pub fn end_screen_stroke(&self) {
        drop_if_unchanged(&self.screen_undo, &self.screen.borrow());
    }

    /// Changes the screen without recording an undo step (used while a stroke is in progress).
    pub fn draw_screen(&self, f: impl FnOnce(&mut Bitmap)) {
        let before = self.screen.borrow().clone();
        f(&mut self.screen.borrow_mut());
        if *self.screen.borrow() != before {
            self.emit(Topic::Screen);
        }
    }

    pub fn undo_screen(&self) -> bool {
        let Some(previous) = self.screen_undo.borrow_mut().pop() else {
            return false;
        };
        *self.screen.borrow_mut() = previous;
        self.emit(Topic::Screen);
        true
    }

    /// Replaces everything at once, e.g. when opening a profile. Undoable per page.
    pub fn load(&self, region: Region, lighting: Lighting, screen: Bitmap) {
        self.set_region(region);
        self.edit_lighting(|l| *l = lighting);
        self.edit_screen(|s| *s = screen);
    }
}

fn push_limited<T>(stack: &RefCell<Vec<T>>, item: T) {
    let mut stack = stack.borrow_mut();
    if stack.len() == UNDO_LIMIT {
        stack.remove(0);
    }
    stack.push(item);
}

fn drop_if_unchanged<T: PartialEq>(stack: &RefCell<Vec<T>>, current: &T) {
    let mut stack = stack.borrow_mut();
    if stack.last() == Some(current) {
        stack.pop();
    }
}
