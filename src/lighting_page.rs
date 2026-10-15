//! The Lighting page: tools, the keyboard view, colors and presets.

use std::collections::BTreeSet;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::editor::{Editor, KeyTool, Topic};
use crate::keyboard_view;
use crate::layout::{self, Key};
use crate::model::Rgb;
use crate::ui::{card, icon_button_content};

pub const SWATCHES: [Rgb; 10] = [
    Rgb::new(0xff, 0x3b, 0x30),
    Rgb::new(0xff, 0x95, 0x00),
    Rgb::new(0xff, 0xcc, 0x00),
    Rgb::new(0x34, 0xc7, 0x59),
    Rgb::new(0x00, 0xc8, 0xff),
    Rgb::new(0x00, 0x5c, 0xff),
    Rgb::new(0xaf, 0x52, 0xde),
    Rgb::new(0xff, 0x2d, 0x95),
    Rgb::new(0xff, 0xff, 0xff),
    Rgb::new(0x00, 0x00, 0x00),
];

pub fn build(editor: &Rc<Editor>) -> gtk::Widget {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    root.append(&toolbar(editor));
    root.append(&keyboard_view::build(editor));

    let bottom = gtk::Box::builder().spacing(14).homogeneous(true).build();
    bottom.append(&color_card(editor));
    bottom.append(&presets_card(editor));
    root.append(&bottom);

    root.upcast()
}

fn toolbar(editor: &Rc<Editor>) -> gtk::Widget {
    let bar = gtk::Box::builder().spacing(12).build();

    let tools = gtk::Box::builder().css_classes(["linked"]).build();
    let mut group: Option<gtk::ToggleButton> = None;
    let mut buttons = Vec::new();
    for (tool, icon, label, tip) in [
        (KeyTool::Select, "edit-select-symbolic", "Select", "Click, Shift-click or drag to select keys"),
        (KeyTool::Paint, "document-edit-symbolic", "Paint", "Click or drag over keys to paint them. Right-click turns keys off"),
        (KeyTool::Pick, "color-select-symbolic", "Pick", "Click a key to take its color"),
    ] {
        let button = gtk::ToggleButton::builder()
            .child(&icon_button_content(icon, label))
            .tooltip_text(tip)
            .active(editor.tool() == tool)
            .build();
        button.set_group(group.as_ref());
        group.get_or_insert_with(|| button.clone());
        button.connect_toggled(glib::clone!(
            #[strong] editor,
            move |b| {
                if b.is_active() {
                    editor.set_tool(tool);
                }
            }
        ));
        tools.append(&button);
        buttons.push((tool, button));
    }
    editor.connect(Topic::Tool, glib::clone!(
        #[strong] editor,
        move || {
            for (tool, button) in &buttons {
                if *tool == editor.tool() && !button.is_active() {
                    button.set_active(true);
                }
            }
        }
    ));
    bar.append(&tools);

    let selection_buttons = gtk::Box::builder().spacing(6).build();
    let select_all = gtk::Button::builder().label("Select all").action_name("win.select-all").build();
    let select_none = gtk::Button::builder().label("Select none").action_name("win.select-none").build();
    let invert = gtk::Button::with_label("Invert selection");
    invert.connect_clicked(glib::clone!(
        #[strong] editor,
        move |_| {
            let selected = editor.selection().clone();
            let inverted: BTreeSet<u8> = editor.keys().iter().map(|k| k.led).filter(|led| !selected.contains(led)).collect();
            editor.set_selection(inverted);
        }
    ));
    selection_buttons.append(&select_all);
    selection_buttons.append(&select_none);
    selection_buttons.append(&invert);
    bar.append(&selection_buttons);

    let count = gtk::Label::builder().hexpand(true).xalign(1.0).css_classes(["dim-label"]).build();
    let update_count = glib::clone!(
        #[weak] count,
        #[strong] editor,
        move || {
            let n = editor.selection().len();
            count.set_label(&match n {
                0 => "No keys selected".to_string(),
                1 => "1 key selected".to_string(),
                n => format!("{n} keys selected"),
            });
        }
    );
    update_count();
    editor.connect(Topic::Selection, update_count);
    bar.append(&count);

    bar.upcast()
}

fn color_card(editor: &Rc<Editor>) -> gtk::Widget {
    let (outer, content) = card("Color", "Painting and filling use this color.");

    let swatches = gtk::Box::builder().spacing(6).build();
    for (i, color) in SWATCHES.iter().copied().enumerate() {
        let button = gtk::Button::builder()
            .css_classes(["swatch", &format!("swatch-{i}")])
            .tooltip_text(color.to_hex())
            .build();
        button.connect_clicked(glib::clone!(#[strong] editor, move |_| editor.set_color(color)));
        swatches.append(&button);
    }
    content.append(&swatches);

    let row = gtk::Box::builder().spacing(8).build();
    let dialog = gtk::ColorDialog::builder().with_alpha(false).title("Key color").build();
    let picker = gtk::ColorDialogButton::new(Some(dialog));
    picker.set_rgba(&to_rgba(editor.color()));
    picker.set_tooltip_text(Some("Choose any color"));
    let hex = gtk::Entry::builder()
        .text(editor.color().to_hex())
        .width_chars(9)
        .max_width_chars(9)
        .tooltip_text("Hex color, like #00c8ff")
        .build();
    let fill = gtk::Button::builder().label("Fill selection").tooltip_text("Fill the selected keys, or all keys when none are selected").build();

    picker.connect_rgba_notify(glib::clone!(
        #[strong] editor,
        move |p| editor.set_color(from_rgba(&p.rgba()))
    ));
    hex.connect_changed(glib::clone!(
        #[strong] editor,
        move |e| match Rgb::parse(&e.text()) {
            Some(c) => {
                e.remove_css_class("error");
                editor.set_color(c);
            }
            None => e.add_css_class("error"),
        }
    ));
    editor.connect(Topic::Color, glib::clone!(
        #[weak] picker,
        #[weak] hex,
        #[strong] editor,
        move || {
            let c = editor.color();
            if from_rgba(&picker.rgba()) != c {
                picker.set_rgba(&to_rgba(c));
            }
            if Rgb::parse(&hex.text()) != Some(c) {
                hex.set_text(&c.to_hex());
            }
        }
    ));
    fill.connect_clicked(glib::clone!(
        #[strong] editor,
        move |_| {
            let color = editor.color();
            let targets = editor.target_leds();
            editor.edit_lighting(|l| targets.iter().for_each(|&led| l.set(led, color)));
        }
    ));

    row.append(&picker);
    row.append(&hex);
    row.append(&gtk::Box::builder().hexpand(true).build());
    row.append(&fill);
    content.append(&row);
    outer.upcast()
}

type Preset = fn(&Key, Rgb) -> Rgb;

const GAMING_KEYS: [u8; 8] = [0x1A, 0x04, 0x16, 0x07, 0x52, 0x50, 0x51, 0x4F]; // WASD and arrows

fn presets_card(editor: &Rc<Editor>) -> gtk::Widget {
    let (outer, content) = card("Presets", "Applied to the selected keys, or to every key when none are selected.");

    let presets: [(&str, &str, Preset); 6] = [
        ("Solid", "Every key in the current color", |_, c| c),
        ("Rainbow", "Hue changes from left to right", |k, _| {
            let (x0, _, x1, _) = k.bounds();
            Rgb::from_hsv((x0 + x1) / 2.0 / layout::WIDTH * 330.0, 1.0, 1.0)
        }),
        ("Rainbow rows", "One hue per row", |k, _| {
            let (_, y0, _, _) = k.bounds();
            Rgb::from_hsv(y0 / layout::HEIGHT * 300.0, 1.0, 1.0)
        }),
        ("Random", "A random hue for each key", |_, _| Rgb::from_hsv(glib::random_double_range(0.0, 360.0), 1.0, 1.0)),
        ("Gaming", "WASD and arrows bright, the rest dimmed", |k, c| {
            if GAMING_KEYS.contains(&k.led) { c } else { Rgb::BLACK.mix(c, 0.15) }
        }),
        ("All off", "Turn the LEDs off", |_, _| Rgb::BLACK),
    ];

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .min_children_per_line(3)
        .max_children_per_line(6)
        .homogeneous(true)
        .build();
    for (name, tip, preset) in presets {
        let button = gtk::Button::builder().label(name).tooltip_text(tip).build();
        button.connect_clicked(glib::clone!(
            #[strong] editor,
            move |_| {
                let color = editor.color();
                let targets = editor.target_leds();
                let keys = editor.keys();
                editor.edit_lighting(|l| {
                    for key in keys.iter().filter(|k| targets.contains(&k.led)) {
                        l.set(key.led, preset(key, color));
                    }
                });
            }
        ));
        flow.insert(&button, -1);
    }
    content.append(&flow);
    outer.upcast()
}

pub fn to_rgba(c: Rgb) -> gdk::RGBA {
    let (r, g, b) = c.to_f64();
    gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0)
}

pub fn from_rgba(c: &gdk::RGBA) -> Rgb {
    let f = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb::new(f(c.red()), f(c.green()), f(c.blue()))
}
