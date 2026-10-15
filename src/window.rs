// SPDX-License-Identifier: GPL-3.0-or-later

//! The main window: header bar, page switcher, menu and window actions.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::editor::{Editor, Topic};
use crate::layout::{self, Region};
use crate::model::{Lighting, Rgb};
use crate::oled_art::{self, Align, TextSpec};
use crate::profile::{self, Profile};
use crate::ui::{icon_button_content, toast};
use crate::{lighting_page, screen_page};

pub fn build(app: &adw::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    let editor = restore_session();

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(&lighting_page::build(&editor), Some("lighting"), "Lighting", "keyboard-brightness-symbolic");
    stack.add_titled_with_icon(&screen_page::build(&editor), Some("screen"), "Screen", "video-display-symbolic");

    let header = adw::HeaderBar::builder()
        .title_widget(&adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build())
        .build();
    header.pack_start(
        &gtk::Button::builder()
            .icon_name("edit-undo-symbolic")
            .action_name("win.undo")
            .tooltip_text("Undo (Ctrl+Z)")
            .build(),
    );

    let menu = gio::Menu::new();
    let files = gio::Menu::new();
    files.append(Some("_Open profile…"), Some("win.open"));
    files.append(Some("_Save profile as…"), Some("win.save"));
    menu.append_section(None, &files);
    let options = gio::Menu::new();
    options.append(Some("_ISO layout (UK, DE, Nordic)"), Some("win.iso-layout"));
    menu.append_section(None, &options);
    let about = gio::Menu::new();
    about.append(Some("_About OpenSteelStudio"), Some("win.about"));
    menu.append_section(None, &about);
    header.pack_end(
        &gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main menu")
            .build(),
    );
    header.pack_end(
        &gtk::Button::builder()
            .child(&icon_button_content("send-to-symbolic", "Apply"))
            .css_classes(["suggested-action"])
            .action_name("win.apply")
            .tooltip_text("Send the lighting and screen to the keyboard")
            .build(),
    );

    let banner = adw::Banner::builder()
        .title("Preview only. The keyboard connection isn't built yet, so nothing is sent to the keyboard.")
        .revealed(true)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&banner);
    toolbar.set_content(Some(&stack));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("OpenSteelStudio")
        .default_width(1180)
        .default_height(820)
        .width_request(900)
        .height_request(640)
        .content(&toasts)
        .build();

    add_actions(&window, &editor, &stack, &toasts);

    // Undo and select all only fire when the focused widget doesn't use the shortcut itself,
    // so they don't steal Ctrl+Z / Ctrl+A from text fields.
    let shortcuts = gtk::ShortcutController::new();
    for (trigger, action) in [("<Control>z", "win.undo"), ("<Control>a", "win.select-all"), ("Escape", "win.select-none")] {
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(trigger),
            Some(gtk::NamedAction::new(action)),
        ));
    }
    window.add_controller(shortcuts);

    window.connect_close_request(glib::clone!(
        #[strong] editor,
        move |_| {
            if let Err(e) = Profile::capture(&editor).save(&profile::session_path()) {
                eprintln!("Couldn't save the session: {e}");
            }
            glib::Propagation::Proceed
        }
    ));

    window.present();

    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("OPENSTEEL_SCREENSHOT") {
        dev_screenshot(&window, &stack, path);
    }
}

/// Development aid: `OPENSTEEL_SCREENSHOT=out.png [OPENSTEEL_PAGE=screen]` renders the window to a PNG and quits.
#[cfg(debug_assertions)]
fn dev_screenshot(window: &adw::ApplicationWindow, stack: &adw::ViewStack, path: String) {
    if let Ok(page) = std::env::var("OPENSTEEL_PAGE") {
        stack.set_visible_child_name(&page);
    }
    let window = window.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer())
            && let Err(e) = renderer.render_texture(&node, None).save_to_png(&path)
        {
            eprintln!("screenshot failed: {e}");
        }
        if let Some(app) = window.application() {
            app.quit();
        }
    });
}

fn add_actions(window: &adw::ApplicationWindow, editor: &Rc<Editor>, stack: &adw::ViewStack, toasts: &adw::ToastOverlay) {
    let action = |name: &str, f: Box<dyn Fn()>| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| f());
        window.add_action(&a);
    };

    action("undo", Box::new(glib::clone!(
        #[strong] editor,
        #[weak] stack,
        move || {
            if stack.visible_child_name().as_deref() == Some("screen") {
                editor.undo_screen();
            } else {
                editor.undo_lighting();
            }
        }
    )));

    action("select-all", Box::new(glib::clone!(
        #[strong] editor,
        move || editor.set_selection(editor.keys().iter().map(|k| k.led).collect())
    )));
    action("select-none", Box::new(glib::clone!(#[strong] editor, move || editor.set_selection(Default::default()))));

    action("apply", Box::new(glib::clone!(
        #[weak] toasts,
        move || toasts.add_toast(adw::Toast::new("Not sent: the keyboard connection comes in the next step"))
    )));

    action("open", Box::new(glib::clone!(
        #[strong] editor,
        #[weak] window,
        move || {
            let dialog = gtk::FileDialog::builder().title("Open profile").modal(true).filters(&json_filters()).build();
            dialog.open(Some(&window), gio::Cancellable::NONE, glib::clone!(
                #[strong] editor,
                #[weak] window,
                move |result| {
                    let Ok(path) = result.map(|f| f.path()) else { return };
                    let Some(path) = path else { return };
                    match Profile::load(&path).and_then(Profile::into_parts) {
                        Ok((region, lighting, screen)) => {
                            editor.load(region, lighting, screen);
                            toast(&window.content().unwrap(), &format!("Opened {}", display_name(&path)));
                        }
                        Err(e) => toast(&window.content().unwrap(), &format!("Couldn't open that profile: {e}")),
                    }
                }
            ));
        }
    )));

    action("save", Box::new(glib::clone!(
        #[strong] editor,
        #[weak] window,
        move || {
            let dialog = gtk::FileDialog::builder()
                .title("Save profile")
                .modal(true)
                .initial_name("apex5-profile.json")
                .filters(&json_filters())
                .build();
            dialog.save(Some(&window), gio::Cancellable::NONE, glib::clone!(
                #[strong] editor,
                #[weak] window,
                move |result| {
                    let Ok(path) = result.map(|f| f.path()) else { return };
                    let Some(path) = path else { return };
                    match Profile::capture(&editor).save(&path) {
                        Ok(()) => toast(&window.content().unwrap(), &format!("Saved {}", display_name(&path))),
                        Err(e) => toast(&window.content().unwrap(), &format!("Couldn't save: {e}")),
                    }
                }
            ));
        }
    )));

    let iso = gio::SimpleAction::new_stateful("iso-layout", None, &(editor.region() == Region::Iso).to_variant());
    iso.connect_change_state(glib::clone!(
        #[strong] editor,
        move |a, value| {
            let Some(value) = value else { return };
            a.set_state(value);
            editor.set_region(if value.get::<bool>() == Some(true) { Region::Iso } else { Region::Ansi });
        }
    ));
    editor.connect(Topic::Layout, glib::clone!(
        #[weak] iso,
        #[strong] editor,
        move || iso.set_state(&(editor.region() == Region::Iso).to_variant())
    ));
    window.add_action(&iso);

    action("about", Box::new(glib::clone!(
        #[weak] window,
        move || {
            adw::AboutDialog::builder()
                .application_name("OpenSteelStudio")
                .application_icon("input-keyboard-symbolic")
                .version(env!("CARGO_PKG_VERSION"))
                .comments("Lighting and OLED screen editor for SteelSeries keyboards.")
                .website("https://github.com/Spider-Frog/OpenSteelStudio")
                .license_type(gtk::License::Gpl30)
                .issue_url("https://github.com/Spider-Frog/OpenSteelStudio/issues")
                .build()
                .present(Some(&window));
        }
    )));
}

fn json_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("OpenSteelStudio profiles (JSON)"));
    filter.add_suffix("json");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    filters
}

fn display_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The previous session, or a rainbow and "APEX 5" on first launch.
fn restore_session() -> Rc<Editor> {
    let path = profile::session_path();
    if path.exists() {
        match Profile::load(&path).and_then(Profile::into_parts) {
            Ok((region, lighting, screen)) => return Editor::new(region, lighting, screen),
            Err(e) => eprintln!("Ignoring the saved session ({}): {e}", path.display()),
        }
    }

    let mut lighting = Lighting::default();
    for key in layout::keys(Region::Ansi).into_iter().chain(layout::keys(Region::Iso)) {
        let (x0, _, x1, _) = key.bounds();
        lighting.set(key.led, Rgb::from_hsv((x0 + x1) / 2.0 / layout::WIDTH * 330.0, 1.0, 1.0));
    }
    let font = gtk::pango::FontDescription::from_string("Sans Bold 16");
    let screen = oled_art::render_text(&TextSpec { text: "APEX 5", font: &font, align: Align::Center, offset: (0, 0) });
    Editor::new(Region::Ansi, lighting, screen)
}
