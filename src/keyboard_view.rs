// SPDX-License-Identifier: GPL-3.0-or-later

//! The interactive keyboard drawing on the Lighting page.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk, glib, pango};

use crate::draw::{self, set_rgb};
use crate::editor::{Editor, KeyTool, Topic};
use crate::layout;
use crate::model::Rgb;

/// Case margin around the keys, in key units.
const PAD: f64 = 0.35;
/// Space between keycaps, in key units.
const GAP: f64 = 0.07;
const CASE: Rgb = Rgb::new(0x1b, 0x1b, 0x1f);
const CAP_OFF: Rgb = Rgb::new(0x2e, 0x2e, 0x34);
const OLED_ON: (f64, f64, f64, f64) = (0.86, 0.94, 1.0, 1.0);

#[derive(Clone, Copy)]
struct Transform {
    unit: f64,
    ox: f64,
    oy: f64,
}

impl Transform {
    fn new(w: f64, h: f64) -> Self {
        let unit = (w / (layout::WIDTH + 2.0 * PAD)).min(h / (layout::HEIGHT + 2.0 * PAD));
        Self { unit, ox: (w - layout::WIDTH * unit) / 2.0, oy: (h - layout::HEIGHT * unit) / 2.0 }
    }

    fn of(area: &gtk::DrawingArea) -> Self {
        Self::new(area.width() as f64, area.height() as f64)
    }

    fn to_keys(self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.ox) / self.unit, (y - self.oy) / self.unit)
    }

    fn to_widget(self, (x, y): (f64, f64)) -> (f64, f64) {
        (self.ox + x * self.unit, self.oy + y * self.unit)
    }
}

struct Drag {
    start: (f64, f64),
    /// Selection when the drag started; a rubber band adds to it.
    base: BTreeSet<u8>,
    rubber: Option<(f64, f64, f64, f64)>,
    erase: bool,
}

pub fn build(editor: &Rc<Editor>) -> gtk::Widget {
    let area = gtk::DrawingArea::builder()
        .content_width(760)
        .content_height(230)
        .hexpand(true)
        .vexpand(true)
        .has_tooltip(true)
        .build();
    let hover: Rc<Cell<Option<u8>>> = Rc::default();
    let drag: Rc<RefCell<Option<Drag>>> = Rc::default();

    area.set_draw_func(glib::clone!(
        #[strong] editor,
        #[strong] hover,
        #[strong] drag,
        move |_, cr, w, h| {
            let rubber = drag.borrow().as_ref().and_then(|d| d.rubber);
            paint(&editor, cr, w as f64, h as f64, hover.get(), rubber);
        }
    ));

    for topic in [Topic::Layout, Topic::Selection, Topic::Frame] {
        editor.connect(topic, glib::clone!(#[weak] area, move || area.queue_draw()));
    }

    let key_at = glib::clone!(
        #[strong] editor,
        move |area: &gtk::DrawingArea, x: f64, y: f64| {
            let (kx, ky) = Transform::of(area).to_keys(x, y);
            editor.keys().iter().find(|k| k.contains(kx, ky)).map(|k| k.led)
        }
    );

    // Hover highlight
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion(glib::clone!(
        #[weak] area,
        #[strong] hover,
        #[strong] key_at,
        move |_, x, y| {
            let led = key_at(&area, x, y);
            if hover.replace(led) != led {
                area.queue_draw();
            }
        }
    ));
    motion.connect_leave(glib::clone!(
        #[weak] area,
        #[strong] hover,
        move |_| {
            if hover.take().is_some() {
                area.queue_draw();
            }
        }
    ));
    area.add_controller(motion);

    // Tooltips name the key under the pointer.
    area.connect_query_tooltip(glib::clone!(
        #[strong] editor,
        move |area, x, y, _keyboard, tooltip| {
            let (kx, ky) = Transform::of(area).to_keys(x as f64, y as f64);
            let keys = editor.keys();
            let Some(key) = keys.iter().find(|k| k.contains(kx, ky)) else {
                return false;
            };
            let color = editor.lighting().get(key.led).to_hex();
            tooltip.set_text(Some(&format!("{}\n{color}", key.name)));
            true
        }
    ));

    // Click, drag and rubber-band selection; painting and color picking.
    let gesture = gtk::GestureDrag::builder().button(0).build();
    gesture.connect_drag_begin(glib::clone!(
        #[weak] area,
        #[strong] editor,
        #[strong] drag,
        #[strong] key_at,
        move |g, x, y| {
            let button = g.current_button();
            let hit = key_at(&area, x, y);
            let start = Transform::of(&area).to_keys(x, y);
            match editor.tool() {
                KeyTool::Select => {
                    if button != gdk::BUTTON_PRIMARY {
                        return;
                    }
                    let modifiers = g.current_event_state();
                    let additive = modifiers.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK);
                    let mut selection = if additive { editor.selection().clone() } else { BTreeSet::new() };
                    if let Some(led) = hit {
                        if additive && selection.contains(&led) {
                            selection.remove(&led);
                        } else {
                            selection.insert(led);
                        }
                    }
                    editor.set_selection(selection.clone());
                    *drag.borrow_mut() = Some(Drag { start, base: selection, rubber: None, erase: false });
                }
                KeyTool::Paint => {
                    let erase = button == gdk::BUTTON_SECONDARY;
                    editor.begin_lighting_stroke();
                    if let Some(led) = hit {
                        editor.paint_key(led, if erase { Rgb::BLACK } else { editor.color() });
                    }
                    *drag.borrow_mut() = Some(Drag { start, base: BTreeSet::new(), rubber: None, erase });
                }
                KeyTool::Pick => {
                    if let Some(led) = hit {
                        let color = editor.lighting().get(led);
                        editor.set_color(color);
                        editor.finish_pick();
                    }
                }
            }
        }
    ));
    gesture.connect_drag_update(glib::clone!(
        #[weak] area,
        #[strong] editor,
        #[strong] drag,
        #[strong] key_at,
        move |g, dx, dy| {
            let Some((sx, sy)) = g.start_point() else { return };
            let (x, y) = (sx + dx, sy + dy);
            let mut state = drag.borrow_mut();
            let Some(d) = state.as_mut() else { return };
            match editor.tool() {
                KeyTool::Select => {
                    if dx.abs() < 4.0 && dy.abs() < 4.0 && d.rubber.is_none() {
                        return;
                    }
                    let (kx, ky) = Transform::of(&area).to_keys(x, y);
                    let rect = (d.start.0.min(kx), d.start.1.min(ky), d.start.0.max(kx), d.start.1.max(ky));
                    d.rubber = Some(rect);
                    let mut selection = d.base.clone();
                    selection.extend(editor.keys().iter().filter(|k| k.intersects(rect)).map(|k| k.led));
                    drop(state);
                    editor.set_selection(selection);
                    area.queue_draw();
                }
                KeyTool::Paint => {
                    let color = if d.erase { Rgb::BLACK } else { editor.color() };
                    drop(state);
                    if let Some(led) = key_at(&area, x, y) {
                        editor.paint_key(led, color);
                    }
                }
                KeyTool::Pick => {}
            }
        }
    ));
    gesture.connect_drag_end(glib::clone!(
        #[weak] area,
        #[strong] editor,
        #[strong] drag,
        move |_, _, _| {
            if drag.take().is_some() && editor.tool() == KeyTool::Paint {
                editor.end_lighting_stroke();
            }
            area.queue_draw();
        }
    ));
    area.add_controller(gesture);

    // Crosshair while picking, a brush-like cursor while painting.
    let update_cursor = glib::clone!(
        #[weak] area,
        #[strong] editor,
        move || {
            let name = match editor.tool() {
                KeyTool::Select => "default",
                KeyTool::Paint => "cell",
                KeyTool::Pick => "crosshair",
            };
            area.set_cursor_from_name(Some(name));
        }
    );
    update_cursor();
    editor.connect(Topic::Tool, update_cursor);

    let ratio = (layout::WIDTH + 2.0 * PAD) / (layout::HEIGHT + 2.0 * PAD);
    let frame = gtk::AspectFrame::new(0.5, 0.5, ratio as f32, false);
    frame.set_child(Some(&area));
    frame.upcast()
}

fn accent() -> (f64, f64, f64) {
    let c = adw::StyleManager::default().accent_color_rgba();
    (c.red() as f64, c.green() as f64, c.blue() as f64)
}

/// How a key looks when lit with `color`: the cap takes on the color, dim colors stay close to an unlit cap.
fn cap_color(color: Rgb) -> Rgb {
    let peak = color.r.max(color.g).max(color.b) as f64 / 255.0;
    CAP_OFF.mix(color, 0.92 * peak.sqrt())
}

fn paint(editor: &Editor, cr: &cairo::Context, w: f64, h: f64, hover: Option<u8>, rubber: Option<(f64, f64, f64, f64)>) {
    let t = Transform::new(w, h);
    let u = t.unit;
    let (ar, ag, ab) = accent();

    // Case
    let (cx, cy) = t.to_widget((-PAD, -PAD));
    draw::rounded_rect(cr, cx, cy, (layout::WIDTH + 2.0 * PAD) * u, (layout::HEIGHT + 2.0 * PAD) * u, 0.3 * u);
    set_rgb(cr, CASE);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    // OLED, showing what the keyboard's screen shows
    let (ox, oy, ow, oh) = layout::OLED;
    let (x, y) = t.to_widget((ox, oy));
    draw::rounded_rect(cr, x, y, ow * u, oh * u, 0.06 * u);
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.fill();
    let m = 0.07 * u;
    draw::bitmap(cr, &editor.live_screen(), (x + m, y + m, ow * u - 2.0 * m, oh * u - 2.0 * m), OLED_ON, (0.0, 0.0, 0.0, 0.0));

    // Media key and volume roller (decorative)
    let (mx, my) = t.to_widget((ox + ow + 0.42, 0.5));
    cr.arc(mx, my, 0.2 * u, 0.0, std::f64::consts::TAU);
    set_rgb(cr, CAP_OFF);
    let _ = cr.fill();
    let (rx, ry) = t.to_widget((ox + ow + 0.74, 0.08));
    draw::rounded_rect(cr, rx, ry, 0.34 * u, 0.84 * u, 0.08 * u);
    cr.set_source_rgb(0.42, 0.42, 0.45);
    let _ = cr.fill();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    cr.set_line_width((0.025 * u).max(1.0));
    for i in 1..8 {
        let ly = ry + i as f64 * 0.84 * u / 8.0;
        cr.move_to(rx + 0.04 * u, ly);
        cr.line_to(rx + 0.30 * u, ly);
    }
    let _ = cr.stroke();

    // Keys
    let keys = editor.keys();
    let lighting = editor.live_lighting();
    let selection = editor.selection();
    let text = pangocairo::functions::create_layout(cr);
    let mut font = pango::FontDescription::from_string("Sans");
    font.set_absolute_size(0.25 * u * pango::SCALE as f64);
    text.set_font_description(Some(&font));

    let outline = |key: &layout::Key, d: f64| {
        let pts: Vec<_> = draw::inset(&key.outline, d).into_iter().map(|p| t.to_widget(p)).collect();
        draw::polygon_path(cr, &pts, 0.12 * u);
    };

    for key in keys.iter() {
        let cap = cap_color(lighting.get(key.led));
        outline(key, GAP / 2.0);
        set_rgb(cr, cap);
        let _ = cr.fill();

        if !key.label.is_empty() {
            text.set_text(key.label);
            let (lw, lh) = text.pixel_size();
            let (lx, ly) = t.to_widget(key.label_at);
            cr.move_to(lx - lw as f64 / 2.0, ly - lh as f64 / 2.0);
            if cap.luma() > 0.55 {
                cr.set_source_rgba(0.08, 0.08, 0.08, 0.9);
            } else {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
            }
            pangocairo::functions::show_layout(cr, &text);
        }
    }

    for key in keys.iter() {
        if selection.contains(&key.led) {
            outline(key, GAP / 2.0 + 0.02);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(0.1 * u);
            let _ = cr.stroke_preserve();
            cr.set_source_rgb(ar, ag, ab);
            cr.set_line_width(0.05 * u);
            let _ = cr.stroke();
        } else if hover == Some(key.led) {
            outline(key, GAP / 2.0 + 0.02);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.6);
            cr.set_line_width(0.04 * u);
            let _ = cr.stroke();
        }
    }

    if let Some((x0, y0, x1, y1)) = rubber {
        let (wx0, wy0) = t.to_widget((x0, y0));
        let (wx1, wy1) = t.to_widget((x1, y1));
        cr.rectangle(wx0, wy0, wx1 - wx0, wy1 - wy0);
        cr.set_source_rgba(ar, ag, ab, 0.18);
        let _ = cr.fill_preserve();
        cr.set_source_rgb(ar, ag, ab);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
}
