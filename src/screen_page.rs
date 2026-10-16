// SPDX-License-Identifier: GPL-3.0-or-later

//! The Screen page: what the 128×40 OLED shows. In Image mode it is a zoomed pixel editor
//! with text and image tools; the other modes animate (clock, scrolling text, GIF).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, pango};

use crate::draw;
use crate::editor::{Editor, Topic};
use crate::lighting_page::{set_choice, set_slider, slider};
use crate::model::{Bitmap, SCREEN_H, SCREEN_W};
use crate::oled_art::{self, Align, Conversion, Fit, Fitted, Gray, TextSpec};
use crate::screen_fx::{Animation, ClockStyle, ScreenMode, ScreenTyping};
use crate::ui::{card, icon_button_content, toast};

const ON: (f64, f64, f64, f64) = (0.86, 0.94, 1.0, 1.0);
const OFF: (f64, f64, f64, f64) = (0.0, 0.0, 0.0, 1.0);

/// A source image kept after importing, so its conversion settings can still be tweaked.
struct Source {
    gray: Gray,
    fit: Fit,
    fitted: Fitted,
}

impl Source {
    fn new(gray: Gray, fit: Fit) -> Self {
        let fitted = oled_art::fit(&gray, fit);
        Self { gray, fit, fitted }
    }

    fn convert(&mut self, opts: Conversion) -> Bitmap {
        if opts.fit != self.fit {
            self.fit = opts.fit;
            self.fitted = oled_art::fit(&self.gray, opts.fit);
        }
        oled_art::quantize(&self.fitted, opts)
    }
}

struct ImageImport {
    source: Source,
    /// What the import produced. Once the screen differs (hand edits), the import is let go.
    result: Bitmap,
}

struct GifImport {
    frames: Vec<Source>,
    delays_ms: Vec<u32>,
    result: Animation,
}

struct Page {
    editor: Rc<Editor>,
    canvas: glib::WeakRef<gtk::DrawingArea>,
    erase: Cell<bool>,
    /// Text that would be stamped, shown in the accent color until it is.
    preview: RefCell<Bitmap>,
    image: RefCell<Option<ImageImport>>,
    gif: RefCell<Option<GifImport>>,
    last_pixel: Cell<Option<(i32, i32)>>,
}

impl Page {
    fn editing(&self) -> bool {
        self.editor.screen_fx().mode == ScreenMode::Image
    }

    /// The text preview isn't editor state, so its changes redraw the canvas directly.
    fn redraw(&self) {
        if let Some(canvas) = self.canvas.upgrade() {
            canvas.queue_draw();
        }
    }
}

pub fn build(editor: &Rc<Editor>) -> gtk::Widget {
    let canvas = gtk::DrawingArea::builder()
        .content_width(SCREEN_W as i32 * 5)
        .content_height(SCREEN_H as i32 * 5)
        .hexpand(true)
        .vexpand(true)
        .build();

    let page = Rc::new(Page {
        editor: editor.clone(),
        canvas: canvas.downgrade(),
        erase: Cell::new(false),
        preview: RefCell::default(),
        image: RefCell::default(),
        gif: RefCell::default(),
        last_pixel: Cell::new(None),
    });

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    root.append(&toolbar(&page));

    let frame = gtk::Box::builder().css_classes(["oled-frame"]).vexpand(true).build();
    let aspect = gtk::AspectFrame::builder()
        .ratio((SCREEN_W as f32 + 4.0) / (SCREEN_H as f32 + 4.0))
        .obey_child(false)
        .hexpand(true)
        .vexpand(true)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .child(&canvas)
        .build();
    frame.append(&aspect);
    root.append(&frame);

    let modes = gtk::Stack::builder().hexpand(true).transition_type(gtk::StackTransitionType::Crossfade).build();
    let image_cards = gtk::Box::builder().spacing(14).build();
    let text = text_card(&page);
    text.set_hexpand(true);
    let image = image_card(&page);
    image.set_hexpand(true);
    image_cards.append(&text);
    image_cards.append(&image);
    modes.add_named(&image_cards, Some("image"));
    modes.add_named(&clock_card(editor), Some("clock"));
    modes.add_named(&marquee_card(editor), Some("marquee"));
    modes.add_named(&animation_card(&page), Some("animation"));

    let bottom = gtk::Box::builder().spacing(14).build();
    bottom.append(&modes);
    bottom.append(&output_card(editor));
    root.append(&bottom);

    canvas.set_draw_func(glib::clone!(
        #[strong] page,
        move |_, cr, w, h| paint_canvas(&page, cr, w as f64, h as f64)
    ));
    for topic in [Topic::Screen, Topic::Frame, Topic::ScreenFx] {
        editor.connect(topic, glib::clone!(#[weak] canvas, move || canvas.queue_draw()));
    }
    attach_pencil(&page, &canvas);

    let show_mode = glib::clone!(
        #[strong] editor,
        #[weak] modes,
        #[weak] canvas,
        move || {
            let mode = editor.screen_fx().mode;
            modes.set_visible_child_name(match mode {
                ScreenMode::Image => "image",
                ScreenMode::Clock => "clock",
                ScreenMode::Marquee => "marquee",
                ScreenMode::Animation => "animation",
            });
            canvas.set_cursor_from_name(Some(if mode == ScreenMode::Image { "crosshair" } else { "default" }));
        }
    );
    show_mode();
    editor.connect(Topic::ScreenFx, show_mode);

    root.upcast()
}

fn toolbar(page: &Rc<Page>) -> gtk::Widget {
    let bar = gtk::Box::builder().spacing(12).build();

    let mode = gtk::DropDown::from_strings(&ScreenMode::NAMES);
    mode.set_tooltip_text(Some("What the screen shows"));
    mode.connect_selected_notify(glib::clone!(
        #[strong] page,
        move |d| page.editor.update_screen_fx(|fx| fx.mode = ScreenMode::ALL[d.selected() as usize])
    ));
    bar.append(&mode);

    let tools = gtk::Box::builder().css_classes(["linked"]).build();
    let pencil = gtk::ToggleButton::builder()
        .child(&icon_button_content("document-edit-symbolic", "Pencil"))
        .tooltip_text("Draw pixels. Right-click erases")
        .active(true)
        .build();
    let eraser = gtk::ToggleButton::builder()
        .child(&icon_button_content("edit-clear-symbolic", "Eraser"))
        .tooltip_text("Erase pixels")
        .group(&pencil)
        .build();
    eraser.connect_toggled(glib::clone!(#[strong] page, move |b| page.erase.set(b.is_active())));
    tools.append(&pencil);
    tools.append(&eraser);
    bar.append(&tools);

    let edits = gtk::Box::builder().spacing(6).build();
    let invert = gtk::Button::builder()
        .child(&icon_button_content("display-brightness-symbolic", "Invert"))
        .tooltip_text("Swap lit and dark pixels")
        .build();
    invert.connect_clicked(glib::clone!(#[strong] page, move |_| page.editor.edit_screen(|s| s.invert())));
    let clear = gtk::Button::builder()
        .child(&icon_button_content("edit-clear-all-symbolic", "Clear"))
        .tooltip_text("Turn every pixel off")
        .build();
    clear.connect_clicked(glib::clone!(#[strong] page, move |_| page.editor.edit_screen(|s| s.clear())));
    edits.append(&invert);
    edits.append(&clear);
    bar.append(&edits);

    let hint = gtk::Label::builder().hexpand(true).xalign(1.0).css_classes(["dim-label"]).build();
    bar.append(&hint);

    let sync = glib::clone!(
        #[strong] page,
        move || {
            let current = page.editor.screen_fx().mode;
            set_choice(&mode, &ScreenMode::ALL, current);
            let editing = current == ScreenMode::Image;
            tools.set_visible(editing);
            edits.set_visible(editing);
            hint.set_label(if editing { "128 × 40 pixels, monochrome" } else { "Live preview" });
        }
    );
    sync();
    page.editor.connect(Topic::ScreenFx, sync);
    bar.upcast()
}

/// Integer zoom and origin that center the 128×40 grid in a w×h area.
fn grid(w: f64, h: f64) -> (f64, f64, f64) {
    let scale = (w / SCREEN_W as f64).min(h / SCREEN_H as f64).floor().max(1.0);
    let ox = ((w - SCREEN_W as f64 * scale) / 2.0).floor();
    let oy = ((h - SCREEN_H as f64 * scale) / 2.0).floor();
    (scale, ox, oy)
}

fn paint_canvas(page: &Page, cr: &cairo::Context, w: f64, h: f64) {
    let (scale, ox, oy) = grid(w, h);
    let rect = (ox, oy, SCREEN_W as f64 * scale, SCREEN_H as f64 * scale);

    if page.editing() {
        // The image being edited, even while a typing display is showing on the keyboard.
        let screen = page.editor.screen();
        draw::bitmap(cr, &screen, rect, ON, OFF);

        // Pending text: only the pixels it would add.
        let mut pending = page.preview.borrow().clone();
        let mut any = false;
        for y in 0..SCREEN_H as i32 {
            for x in 0..SCREEN_W as i32 {
                let new = pending.get(x, y) && !screen.get(x, y);
                pending.set(x, y, new);
                any |= new;
            }
        }
        if any {
            let c = adw::StyleManager::default().accent_color_rgba();
            draw::bitmap(cr, &pending, rect, (c.red() as f64, c.green() as f64, c.blue() as f64, 1.0), (0.0, 0.0, 0.0, 0.0));
        }
    } else {
        draw::bitmap(cr, &page.editor.live_screen(), rect, ON, OFF);
    }

    if scale >= 6.0 {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.07);
        cr.set_line_width(1.0);
        for x in 1..SCREEN_W {
            let px = ox + x as f64 * scale + 0.5;
            cr.move_to(px, oy);
            cr.line_to(px, oy + rect.3);
        }
        for y in 1..SCREEN_H {
            let py = oy + y as f64 * scale + 0.5;
            cr.move_to(ox, py);
            cr.line_to(ox + rect.2, py);
        }
        let _ = cr.stroke();
    }
}

fn attach_pencil(page: &Rc<Page>, canvas: &gtk::DrawingArea) {
    let pixel_at = |area: &gtk::DrawingArea, x: f64, y: f64| {
        let (scale, ox, oy) = grid(area.width() as f64, area.height() as f64);
        (((x - ox) / scale).floor() as i32, ((y - oy) / scale).floor() as i32)
    };
    let gesture = gtk::GestureDrag::builder().button(0).build();
    gesture.connect_drag_begin(glib::clone!(
        #[strong] page,
        #[weak] canvas,
        move |g, x, y| {
            if !page.editing() {
                return;
            }
            let p = pixel_at(&canvas, x, y);
            let on = !page.erase.get() && g.current_button() != gdk::BUTTON_SECONDARY;
            page.editor.begin_screen_stroke();
            page.editor.draw_screen(|s| s.set(p.0, p.1, on));
            page.last_pixel.set(Some(p));
        }
    ));
    gesture.connect_drag_update(glib::clone!(
        #[strong] page,
        #[weak] canvas,
        move |g, dx, dy| {
            let (Some((sx, sy)), Some(last)) = (g.start_point(), page.last_pixel.get()) else { return };
            let p = pixel_at(&canvas, sx + dx, sy + dy);
            if p == last {
                return;
            }
            let on = !page.erase.get() && g.current_button() != gdk::BUTTON_SECONDARY;
            page.editor.draw_screen(|s| s.line(last, p, on));
            page.last_pixel.set(Some(p));
        }
    ));
    gesture.connect_drag_end(glib::clone!(
        #[strong] page,
        move |_, _, _| {
            if page.last_pixel.take().is_some() {
                page.editor.end_screen_stroke();
            }
        }
    ));
    canvas.add_controller(gesture);
}

fn font_button(title: &str, font: &str) -> gtk::FontDialogButton {
    let button = gtk::FontDialogButton::new(Some(gtk::FontDialog::builder().title(title).build()));
    button.set_font_desc(&pango::FontDescription::from_string(font));
    button.set_tooltip_text(Some("Font and size"));
    button
}

fn font_string(button: &gtk::FontDialogButton) -> Option<String> {
    button.font_desc().map(|f| f.to_str().to_string())
}

/// Two-column label/control grid, as used by the mode cards.
fn form(rows: &[(&str, &gtk::Widget)]) -> gtk::Grid {
    let grid = gtk::Grid::builder().row_spacing(8).column_spacing(12).build();
    for (i, (name, widget)) in rows.iter().enumerate() {
        grid.attach(&gtk::Label::builder().label(*name).xalign(0.0).css_classes(["dim-label"]).build(), 0, i as i32, 1, 1);
        // Sliders use the full width; everything else keeps its natural size.
        if !widget.is::<gtk::Scale>() {
            widget.set_halign(gtk::Align::Start);
        }
        grid.attach(*widget, 1, i as i32, 1, 1);
    }
    grid
}

fn text_card(page: &Rc<Page>) -> gtk::Widget {
    let (outer, content) = card("Text", "Type to preview in color, then stamp it onto the screen.");

    let buffer = gtk::TextBuffer::new(None);
    let view = gtk::TextView::builder()
        .buffer(&buffer)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(8)
        .right_margin(8)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(54)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .css_classes(["card"])
        .build();
    content.append(&scroller);

    let font = font_button("Screen font", "Sans Bold 14");
    let align = gtk::DropDown::from_strings(&["Left", "Center", "Right"]);
    align.set_selected(1);
    align.set_tooltip_text(Some("Alignment"));
    let row = gtk::Box::builder().spacing(8).build();
    row.append(&font);
    row.append(&align);
    content.append(&row);

    let off_x = gtk::SpinButton::with_range(-128.0, 128.0, 1.0);
    off_x.set_value(0.0);
    off_x.set_tooltip_text(Some("Move right (pixels)"));
    let off_y = gtk::SpinButton::with_range(-40.0, 40.0, 1.0);
    off_y.set_value(0.0);
    off_y.set_tooltip_text(Some("Move down (pixels)"));
    let stamp = gtk::Button::builder().child(&icon_button_content("insert-text-symbolic", "Stamp text")).build();
    let row = gtk::Box::builder().spacing(8).build();
    row.append(&gtk::Label::builder().label("X").css_classes(["dim-label"]).build());
    row.append(&off_x);
    row.append(&gtk::Label::builder().label("Y").css_classes(["dim-label"]).build());
    row.append(&off_y);
    row.append(&gtk::Box::builder().hexpand(true).build());
    row.append(&stamp);
    content.append(&row);

    let update = Rc::new(glib::clone!(
        #[strong] page,
        #[weak] buffer,
        #[weak] font,
        #[weak] align,
        #[weak] off_x,
        #[weak] off_y,
        move || {
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let font = font.font_desc().unwrap_or_else(|| pango::FontDescription::from_string("Sans Bold 14"));
            let align = match align.selected() {
                0 => Align::Left,
                2 => Align::Right,
                _ => Align::Center,
            };
            let spec = TextSpec { text: &text, font: &font, align, offset: (off_x.value_as_int(), off_y.value_as_int()) };
            *page.preview.borrow_mut() = oled_art::render_text(&spec);
            page.redraw();
        }
    ));
    buffer.connect_changed(glib::clone!(#[strong] update, move |_| update()));
    font.connect_font_desc_notify(glib::clone!(#[strong] update, move |_| update()));
    align.connect_selected_notify(glib::clone!(#[strong] update, move |_| update()));
    off_x.connect_value_changed(glib::clone!(#[strong] update, move |_| update()));
    off_y.connect_value_changed(glib::clone!(#[strong] update, move |_| update()));

    stamp.connect_clicked(glib::clone!(
        #[strong] page,
        #[weak] buffer,
        move |b| {
            let text = page.preview.borrow().clone();
            if text == Bitmap::default() {
                toast(b, "Type some text first");
                return;
            }
            page.editor.edit_screen(|s| s.merge(&text));
            buffer.set_text("");
        }
    ));

    outer.upcast()
}

/// Fit, dither, invert and brightness controls shared by image and GIF import.
#[derive(Clone)]
struct ToneControls {
    fit: gtk::DropDown,
    dither: gtk::CheckButton,
    invert: gtk::CheckButton,
    brightness: gtk::Scale,
}

impl ToneControls {
    fn new(content: &gtk::Box) -> Self {
        let fit = gtk::DropDown::from_strings(&["Fit", "Fill", "Stretch"]);
        fit.set_tooltip_text(Some("Fit shows the whole image, Fill crops it to cover the screen"));
        let dither = gtk::CheckButton::builder()
            .label("Dither")
            .active(true)
            .tooltip_text("Simulate gray tones with dot patterns")
            .build();
        let invert = gtk::CheckButton::builder().label("Invert").build();
        let row = gtk::Box::builder().spacing(10).build();
        row.append(&fit);
        row.append(&dither);
        row.append(&invert);
        content.append(&row);

        let brightness = gtk::Scale::with_range(gtk::Orientation::Horizontal, -128.0, 128.0, 1.0);
        brightness.set_value(0.0);
        brightness.set_hexpand(true);
        brightness.add_mark(0.0, gtk::PositionType::Bottom, None);
        let row = gtk::Box::builder().spacing(8).build();
        row.append(&gtk::Label::builder().label("Brightness").css_classes(["dim-label"]).build());
        row.append(&brightness);
        content.append(&row);

        Self { fit, dither, invert, brightness }
    }

    fn options(&self) -> Conversion {
        Conversion {
            fit: match self.fit.selected() {
                1 => Fit::Cover,
                2 => Fit::Stretch,
                _ => Fit::Contain,
            },
            dither: self.dither.is_active(),
            brightness: self.brightness.value() as i32,
            invert: self.invert.is_active(),
        }
    }

    fn connect_changed(&self, f: impl Fn() + 'static) {
        let f = Rc::new(f);
        self.fit.connect_selected_notify(glib::clone!(#[strong] f, move |_| f()));
        self.dither.connect_toggled(glib::clone!(#[strong] f, move |_| f()));
        self.invert.connect_toggled(glib::clone!(#[strong] f, move |_| f()));
        self.brightness.connect_value_changed(move |_| f());
    }
}

fn open_dialog(button: &gtk::Button, title: &str, filter: gtk::FileFilter, done: impl FnOnce(gio::File) + 'static) {
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder().title(title).modal(true).filters(&filters).build();
    let window = button.root().and_downcast::<gtk::Window>();
    dialog.open(window.as_ref(), gio::Cancellable::NONE, move |result| {
        if let Ok(file) = result {
            done(file);
        }
    });
}

fn image_card(page: &Rc<Page>) -> gtk::Widget {
    let (outer, content) = card("Image", "Imported images are scaled and converted to black and white.");

    let open = gtk::Button::builder()
        .child(&icon_button_content("insert-image-symbolic", "Import image…"))
        .halign(gtk::Align::Start)
        .build();
    content.append(&open);
    let tone = ToneControls::new(&content);

    // Re-run the conversion while the screen still shows the untouched import.
    tone.connect_changed(glib::clone!(
        #[strong] page,
        #[strong] tone,
        move || {
            let mut import = page.image.borrow_mut();
            let Some(current) = import.as_mut() else { return };
            if *page.editor.screen() != current.result {
                *import = None;
                return;
            }
            let bitmap = current.source.convert(tone.options());
            current.result = bitmap.clone();
            drop(import);
            page.editor.draw_screen(|s| *s = bitmap);
        }
    ));

    open.connect_clicked(glib::clone!(
        #[strong] page,
        #[strong] tone,
        move |button| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_mime_type("image/*");
            open_dialog(button, "Import image", filter, glib::clone!(
                #[strong] page,
                #[strong] tone,
                #[weak] button,
                move |file| match oled_art::load(&file) {
                    Ok(gray) => {
                        let opts = tone.options();
                        let mut source = Source::new(gray, opts.fit);
                        let bitmap = source.convert(opts);
                        page.editor.edit_screen(|s| *s = bitmap.clone());
                        *page.image.borrow_mut() = Some(ImageImport { source, result: bitmap });
                    }
                    Err(e) => toast(&button, &format!("Couldn't open that image: {}", e.message())),
                }
            ));
        }
    ));

    outer.upcast()
}

fn clock_card(editor: &Rc<Editor>) -> gtk::Widget {
    let (outer, content) = card("Clock", "Shows the time from your computer.");
    let style = gtk::DropDown::from_strings(&ClockStyle::NAMES);
    let h24 = gtk::CheckButton::with_label("24-hour");
    let font = font_button("Clock font", &editor.screen_fx().clock_font);
    content.append(&form(&[("Show", style.upcast_ref()), ("Format", h24.upcast_ref()), ("Font", font.upcast_ref())]));

    style.connect_selected_notify(glib::clone!(
        #[strong] editor,
        move |d| editor.update_screen_fx(|fx| fx.clock_style = ClockStyle::ALL[d.selected() as usize])
    ));
    h24.connect_toggled(glib::clone!(#[strong] editor, move |c| editor.update_screen_fx(|fx| fx.clock_24h = c.is_active())));
    font.connect_font_desc_notify(glib::clone!(
        #[strong] editor,
        move |b| {
            if let Some(f) = font_string(b) {
                editor.update_screen_fx(|fx| fx.clock_font = f);
            }
        }
    ));

    let sync = glib::clone!(
        #[strong] editor,
        move || {
            let fx = editor.screen_fx();
            set_choice(&style, &ClockStyle::ALL, fx.clock_style);
            if h24.is_active() != fx.clock_24h {
                h24.set_active(fx.clock_24h);
            }
            if font_string(&font).as_deref() != Some(&fx.clock_font) {
                font.set_font_desc(&pango::FontDescription::from_string(&fx.clock_font));
            }
        }
    );
    sync();
    editor.connect(Topic::ScreenFx, sync);
    outer.upcast()
}

fn marquee_card(editor: &Rc<Editor>) -> gtk::Widget {
    let (outer, content) = card("Scrolling text", "Scrolls from right to left, over and over.");
    let text = gtk::Entry::builder().placeholder_text("Text to scroll").build();
    let font = font_button("Scrolling text font", &editor.screen_fx().marquee_font);
    let speed = slider(0.2, 3.0, |v| format!("{v:.1}×"));
    content.append(&text);
    content.append(&form(&[("Font", font.upcast_ref()), ("Speed", speed.upcast_ref())]));

    text.connect_changed(glib::clone!(
        #[strong] editor,
        move |e| editor.update_screen_fx(|fx| fx.marquee_text = e.text().to_string())
    ));
    font.connect_font_desc_notify(glib::clone!(
        #[strong] editor,
        move |b| {
            if let Some(f) = font_string(b) {
                editor.update_screen_fx(|fx| fx.marquee_font = f);
            }
        }
    ));
    speed.connect_value_changed(glib::clone!(#[strong] editor, move |s| editor.update_screen_fx(|fx| fx.marquee_speed = s.value())));

    let sync = glib::clone!(
        #[strong] editor,
        move || {
            let fx = editor.screen_fx();
            if text.text() != fx.marquee_text {
                text.set_text(&fx.marquee_text);
            }
            if font_string(&font).as_deref() != Some(&fx.marquee_font) {
                font.set_font_desc(&pango::FontDescription::from_string(&fx.marquee_font));
            }
            set_slider(&speed, fx.marquee_speed);
        }
    );
    sync();
    editor.connect(Topic::ScreenFx, sync);
    outer.upcast()
}

fn animation_card(page: &Rc<Page>) -> gtk::Widget {
    let editor = &page.editor;
    let (outer, content) = card("Animation", "Plays an animated GIF in black and white, on a loop.");

    let open = gtk::Button::builder().child(&icon_button_content("insert-image-symbolic", "Import GIF…")).build();
    let info = gtk::Label::builder().xalign(0.0).css_classes(["dim-label"]).build();
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&open);
    row.append(&info);
    content.append(&row);
    let speed = slider(0.2, 3.0, |v| format!("{v:.1}×"));
    content.append(&form(&[("Speed", speed.upcast_ref())]));
    let tone = ToneControls::new(&content);

    speed.connect_value_changed(glib::clone!(#[strong] editor, move |s| editor.update_screen_fx(|fx| fx.animation_speed = s.value())));

    // Reconvert every frame while the editor still holds this import's result.
    tone.connect_changed(glib::clone!(
        #[strong] page,
        #[strong] tone,
        move || {
            let mut import = page.gif.borrow_mut();
            let Some(current) = import.as_mut() else { return };
            if *page.editor.animation() != current.result {
                *import = None;
                return;
            }
            let opts = tone.options();
            let frames = current.frames.iter_mut().map(|f| f.convert(opts)).collect();
            current.result = Animation { frames, delays_ms: current.delays_ms.clone() };
            let animation = current.result.clone();
            drop(import);
            page.editor.set_animation(animation);
        }
    ));

    open.connect_clicked(glib::clone!(
        #[strong] page,
        #[strong] tone,
        move |button| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("GIF animations"));
            filter.add_mime_type("image/gif");
            open_dialog(button, "Import GIF", filter, glib::clone!(
                #[strong] page,
                #[strong] tone,
                #[weak] button,
                move |file| {
                    let Some(path) = file.path() else { return };
                    match oled_art::load_gif(&path) {
                        Ok(decoded) => {
                            let opts = tone.options();
                            let (mut frames, delays_ms): (Vec<Source>, Vec<u32>) =
                                decoded.into_iter().map(|(gray, ms)| (Source::new(gray, opts.fit), ms)).unzip();
                            let result = Animation { frames: frames.iter_mut().map(|f| f.convert(opts)).collect(), delays_ms: delays_ms.clone() };
                            page.editor.set_animation(result.clone());
                            page.editor.update_screen_fx(|fx| fx.mode = ScreenMode::Animation);
                            *page.gif.borrow_mut() = Some(GifImport { frames, delays_ms, result });
                        }
                        Err(e) => toast(&button, &format!("Couldn't open that GIF: {e}")),
                    }
                }
            ));
        }
    ));

    let sync = glib::clone!(
        #[strong] editor,
        move || {
            set_slider(&speed, editor.screen_fx().animation_speed);
            let animation = editor.animation();
            info.set_label(&if animation.frames.is_empty() {
                "No animation yet".to_string()
            } else {
                format!("{} frames, {:.1} s", animation.frames.len(), animation.duration_ms() as f64 / 1000.0)
            });
        }
    );
    sync();
    editor.connect(Topic::ScreenFx, sync);
    outer.upcast()
}

/// What the keyboard's screen shows right now, and the typing display settings.
fn output_card(editor: &Rc<Editor>) -> gtk::Widget {
    let (outer, content) = card("On the keyboard", "");
    let area = gtk::DrawingArea::builder()
        .content_width(SCREEN_W as i32)
        .content_height(SCREEN_H as i32)
        .halign(gtk::Align::Center)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();
    area.set_draw_func(glib::clone!(
        #[strong] editor,
        move |_, cr, _, _| draw::bitmap(cr, &editor.live_screen(), (0.0, 0.0, SCREEN_W as f64, SCREEN_H as f64), ON, OFF)
    ));
    editor.connect(Topic::Frame, glib::clone!(#[weak] area, move || area.queue_draw()));
    let frame = gtk::Box::builder().css_classes(["oled-frame"]).halign(gtk::Align::Center).build();
    frame.append(&area);
    content.append(&frame);

    let typing = gtk::DropDown::from_strings(&ScreenTyping::NAMES);
    typing.set_tooltip_text(Some("Shown for a few seconds after you press a key"));
    let hold = slider(1.0, 10.0, |v| format!("{v:.0} s"));
    hold.set_increments(1.0, 1.0);
    let hold_label = gtk::Label::builder().label("For").xalign(0.0).css_classes(["dim-label"]).build();
    let grid = form(&[("While typing", typing.upcast_ref())]);
    grid.attach(&hold_label, 0, 1, 1, 1);
    grid.attach(&hold, 1, 1, 1, 1);
    content.append(&grid);

    typing.connect_selected_notify(glib::clone!(
        #[strong] editor,
        move |d| editor.update_screen_fx(|fx| fx.typing = ScreenTyping::ALL[d.selected() as usize])
    ));
    hold.connect_value_changed(glib::clone!(#[strong] editor, move |s| editor.update_screen_fx(|fx| fx.typing_hold = s.value().round())));

    let sync = glib::clone!(
        #[strong] editor,
        move || {
            let fx = editor.screen_fx();
            set_choice(&typing, &ScreenTyping::ALL, fx.typing);
            set_slider(&hold, fx.typing_hold);
            let on = fx.typing != ScreenTyping::Off;
            hold.set_visible(on);
            hold_label.set_visible(on);
        }
    );
    sync();
    editor.connect(Topic::ScreenFx, sync);
    outer.set_width_request(250);
    outer.upcast()
}
