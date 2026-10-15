// SPDX-License-Identifier: GPL-3.0-or-later

//! The Screen page: a zoomed pixel editor for the 128×40 OLED, plus text and image tools.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, pango};

use crate::draw;
use crate::editor::{Editor, Topic};
use crate::model::{Bitmap, SCREEN_H, SCREEN_W};
use crate::oled_art::{self, Align, Conversion, Fit, Gray, TextSpec};
use crate::ui::{card, icon_button_content, toast};

const ON: (f64, f64, f64, f64) = (0.86, 0.94, 1.0, 1.0);
const OFF: (f64, f64, f64, f64) = (0.0, 0.0, 0.0, 1.0);

/// The last imported image, kept so the image settings can be tweaked afterwards.
struct Import {
    gray: Gray,
    /// What the import produced. Once the screen differs (hand edits), the import is let go.
    result: Bitmap,
}

struct Page {
    editor: Rc<Editor>,
    canvas: glib::WeakRef<gtk::DrawingArea>,
    erase: Cell<bool>,
    /// Text that would be stamped, shown in the accent color until it is.
    preview: RefCell<Bitmap>,
    import: RefCell<Option<Import>>,
    last_pixel: Cell<Option<(i32, i32)>>,
}

pub fn build(editor: &Rc<Editor>) -> gtk::Widget {
    let canvas = gtk::DrawingArea::builder()
        .content_width(SCREEN_W as i32 * 5)
        .content_height(SCREEN_H as i32 * 5)
        .hexpand(true)
        .vexpand(true)
        .build();
    canvas.set_cursor_from_name(Some("crosshair"));

    let page = Rc::new(Page {
        editor: editor.clone(),
        canvas: canvas.downgrade(),
        erase: Cell::new(false),
        preview: RefCell::default(),
        import: RefCell::default(),
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

    let bottom = gtk::Box::builder().spacing(14).build();
    let text = text_card(&page);
    text.set_hexpand(true);
    let image = image_card(&page);
    image.set_hexpand(true);
    bottom.append(&text);
    bottom.append(&image);
    bottom.append(&preview_card(&page));
    root.append(&bottom);

    canvas.set_draw_func(glib::clone!(
        #[strong] page,
        move |_, cr, w, h| paint_canvas(&page, cr, w as f64, h as f64)
    ));
    editor.connect(Topic::Screen, glib::clone!(#[weak] canvas, move || canvas.queue_draw()));
    attach_pencil(&page, &canvas);

    root.upcast()
}

fn toolbar(page: &Rc<Page>) -> gtk::Widget {
    let bar = gtk::Box::builder().spacing(12).build();

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

    let hint = gtk::Label::builder()
        .label("128 × 40 pixels, monochrome")
        .hexpand(true)
        .xalign(1.0)
        .css_classes(["dim-label"])
        .build();
    bar.append(&hint);
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
            page.last_pixel.set(None);
            page.editor.end_screen_stroke();
        }
    ));
    canvas.add_controller(gesture);
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

    let font_button = gtk::FontDialogButton::new(Some(gtk::FontDialog::builder().title("Screen font").build()));
    font_button.set_font_desc(&pango::FontDescription::from_string("Sans Bold 14"));
    font_button.set_tooltip_text(Some("Font and size"));
    let align = gtk::DropDown::from_strings(&["Left", "Center", "Right"]);
    align.set_selected(1);
    align.set_tooltip_text(Some("Alignment"));
    let off_x = gtk::SpinButton::with_range(-128.0, 128.0, 1.0);
    off_x.set_value(0.0);
    off_x.set_tooltip_text(Some("Move right (pixels)"));
    let off_y = gtk::SpinButton::with_range(-40.0, 40.0, 1.0);
    off_y.set_value(0.0);
    off_y.set_tooltip_text(Some("Move down (pixels)"));

    let row = gtk::Box::builder().spacing(8).build();
    row.append(&font_button);
    row.append(&align);
    row.append(&gtk::Label::builder().label("X").css_classes(["dim-label"]).margin_start(4).build());
    row.append(&off_x);
    row.append(&gtk::Label::builder().label("Y").css_classes(["dim-label"]).build());
    row.append(&off_y);
    content.append(&row);

    let stamp = gtk::Button::builder()
        .child(&icon_button_content("insert-text-symbolic", "Stamp text"))
        .halign(gtk::Align::Start)
        .build();
    content.append(&stamp);

    let update = Rc::new(glib::clone!(
        #[strong] page,
        #[weak] buffer,
        #[weak] font_button,
        #[weak] align,
        #[weak] off_x,
        #[weak] off_y,
        move || {
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let font = font_button.font_desc().unwrap_or_else(|| pango::FontDescription::from_string("Sans Bold 14"));
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
    font_button.connect_font_desc_notify(glib::clone!(#[strong] update, move |_| update()));
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

    // Render the initial preview once the widgets exist.
    glib::idle_add_local_once(move || update());
    outer.upcast()
}

impl Page {
    /// The text preview isn't editor state, so its changes redraw the canvas directly.
    fn redraw(&self) {
        if let Some(canvas) = self.canvas.upgrade() {
            canvas.queue_draw();
        }
    }
}

fn image_card(page: &Rc<Page>) -> gtk::Widget {
    let (outer, content) = card("Image", "Imported images are scaled to fit and converted to black and white.");

    let open = gtk::Button::builder()
        .child(&icon_button_content("insert-image-symbolic", "Import image…"))
        .halign(gtk::Align::Start)
        .build();
    content.append(&open);

    let fit = gtk::DropDown::from_strings(&["Fit", "Fill", "Stretch"]);
    fit.set_tooltip_text(Some("Fit shows the whole image, Fill crops it to cover the screen"));
    let dither = gtk::CheckButton::builder().label("Dither").active(true).tooltip_text("Simulate gray tones with dot patterns").build();
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
    let brightness_row = gtk::Box::builder().spacing(8).build();
    brightness_row.append(&gtk::Label::builder().label("Brightness").css_classes(["dim-label"]).build());
    brightness_row.append(&brightness);
    content.append(&brightness_row);

    let options = Rc::new(glib::clone!(
        #[weak] fit,
        #[weak] dither,
        #[weak] invert,
        #[weak] brightness,
        #[upgrade_or] Conversion { fit: Fit::Contain, dither: true, brightness: 0, invert: false },
        move || Conversion {
            fit: match fit.selected() {
                1 => Fit::Cover,
                2 => Fit::Stretch,
                _ => Fit::Contain,
            },
            dither: dither.is_active(),
            brightness: brightness.value() as i32,
            invert: invert.is_active(),
        }
    ));

    // Re-run the conversion while the screen still shows the untouched import.
    let reapply = Rc::new(glib::clone!(
        #[strong] page,
        #[strong] options,
        move || {
            let mut import = page.import.borrow_mut();
            let Some(current) = import.as_mut() else { return };
            if *page.editor.screen() != current.result {
                *import = None;
                return;
            }
            let bitmap = oled_art::convert(&current.gray, options());
            current.result = bitmap.clone();
            drop(import);
            page.editor.draw_screen(|s| *s = bitmap);
        }
    ));
    fit.connect_selected_notify(glib::clone!(#[strong] reapply, move |_| reapply()));
    dither.connect_toggled(glib::clone!(#[strong] reapply, move |_| reapply()));
    invert.connect_toggled(glib::clone!(#[strong] reapply, move |_| reapply()));
    brightness.connect_value_changed(glib::clone!(#[strong] reapply, move |_| reapply()));

    open.connect_clicked(glib::clone!(
        #[strong] page,
        #[strong] options,
        move |button| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_mime_type("image/*");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Import image").modal(true).filters(&filters).build();
            let window = button.root().and_downcast::<gtk::Window>();
            dialog.open(window.as_ref(), gio::Cancellable::NONE, glib::clone!(
                #[strong] page,
                #[strong] options,
                #[weak] button,
                move |result| {
                    let Ok(file) = result else { return }; // cancelled
                    match oled_art::load(&file) {
                        Ok(gray) => {
                            let bitmap = oled_art::convert(&gray, options());
                            page.editor.edit_screen(|s| *s = bitmap.clone());
                            *page.import.borrow_mut() = Some(Import { gray, result: bitmap });
                        }
                        Err(e) => toast(&button, &format!("Couldn't open that image: {}", e.message())),
                    }
                }
            ));
        }
    ));

    outer.upcast()
}

fn preview_card(page: &Rc<Page>) -> gtk::Widget {
    let (outer, content) = card("Actual size", "");
    let area = gtk::DrawingArea::builder()
        .content_width(SCREEN_W as i32)
        .content_height(SCREEN_H as i32)
        .halign(gtk::Align::Center)
        .build();
    area.set_draw_func(glib::clone!(
        #[strong] page,
        move |_, cr, _, _| {
            draw::bitmap(cr, &page.editor.screen(), (0.0, 0.0, SCREEN_W as f64, SCREEN_H as f64), ON, OFF);
        }
    ));
    page.editor.connect(Topic::Screen, glib::clone!(#[weak] area, move || area.queue_draw()));
    let frame = gtk::Box::builder().css_classes(["oled-frame"]).halign(gtk::Align::Center).build();
    area.set_margin_top(8);
    area.set_margin_bottom(8);
    area.set_margin_start(8);
    area.set_margin_end(8);
    frame.append(&area);
    content.append(&frame);
    outer.upcast()
}
