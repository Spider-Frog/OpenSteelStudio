// SPDX-License-Identifier: GPL-3.0-or-later

//! The main window: header bar, page switcher, menu and window actions.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::editor::{Document, Editor, Topic};
use crate::layout::{self, Region};
use crate::model::{Lighting, Rgb};
use crate::oled_art::{self, Align, TextSpec};
use crate::profile::{self, Profile};
use crate::ui::{icon_button_content, toast};
use crate::device::{self, Link, Status};
use crate::profile::Settings;
use crate::{desktop, engine, keymap, lighting_page, locks, screen_page, volume};

const ACCESS_RULES: &str = include_str!("../data/70-open-steel-studio.rules");

/// Set by `--background`: the first activation keeps the window hidden.
static START_HIDDEN: AtomicBool = AtomicBool::new(false);

pub fn start_hidden() {
    START_HIDDEN.store(true, Ordering::Relaxed);
}

pub fn build(app: &adw::Application) {
    // Launching the app again, or clicking its notification, shows the (maybe hidden) window.
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    let hidden = START_HIDDEN.swap(false, Ordering::Relaxed);

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
    options.append(Some("_Give lighting back to the keyboard"), Some("win.restore-lighting"));
    options.append(Some("Start at _login"), Some("win.autostart"));
    menu.append_section(None, &options);
    let about = gio::Menu::new();
    about.append(Some("_About OpenSteelStudio"), Some("win.about"));
    about.append(Some("_Quit"), Some("app.quit"));
    menu.append_section(None, &about);
    header.pack_end(
        &gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main menu")
            .build(),
    );
    let send = gtk::ToggleButton::builder()
        .child(&icon_button_content("send-to-symbolic", "Send to keyboard"))
        .tooltip_text("Show the lighting and screen on the keyboard, live")
        .build();
    header.pack_end(&send);

    let banner = adw::Banner::builder().use_markup(false).build();

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

    let link = Link::new(&editor);
    add_actions(&window, &editor, &link, &stack, &toasts);
    engine::start(&editor);
    volume::follow(&editor);
    locks::follow(&editor);
    preview_typing(&window, &editor);
    connect_keyboard(&window, &link, &send, &banner, &toasts);

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

    add_app_actions(app, &window, &editor);

    // While sending to the keyboard, closing the window keeps the app running in the
    // background, so animations and typing effects carry on.
    window.connect_close_request(glib::clone!(
        #[strong] editor,
        #[strong] link,
        #[weak] app,
        #[upgrade_or] glib::Propagation::Proceed,
        move |window| {
            save_session(&editor);
            if !link.is_enabled() {
                return glib::Propagation::Proceed;
            }
            window.set_visible(false);
            note_running_in_background(&app);
            glib::Propagation::Stop
        }
    ));

    let sending = Settings::load().send_to_keyboard;
    if hidden && !sending {
        // Started at login with nothing to send: nothing to do.
        window.destroy();
        return;
    }
    if !hidden {
        window.present();
    }
    if sending {
        send.set_active(true);
    }

    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("OPENSTEEL_SCREENSHOT") {
        dev_screenshot(&window, &stack, &editor, path);
    }
    // Development aid: `OPENSTEEL_VOLUME=0.65` pretends the volume just changed to 65 %.
    #[cfg(debug_assertions)]
    if let Some(level) = std::env::var("OPENSTEEL_VOLUME").ok().and_then(|v| v.parse().ok()) {
        glib::timeout_add_local_once(std::time::Duration::from_millis(1000), glib::clone!(
            #[strong] editor,
            move || editor.set_volume(level, false, true)
        ));
    }
    // Development aid: `OPENSTEEL_CLOSE=1` closes the window shortly after start.
    #[cfg(debug_assertions)]
    if std::env::var_os("OPENSTEEL_CLOSE").is_some() {
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), glib::clone!(#[weak] window, move || window.close()));
    }
}

/// Development aid: `OPENSTEEL_SCREENSHOT=out.png [OPENSTEEL_PAGE=screen] [OPENSTEEL_TYPE=text]`
/// renders the window to a PNG and quits, optionally after simulating typing.
#[cfg(debug_assertions)]
fn dev_screenshot(window: &adw::ApplicationWindow, stack: &adw::ViewStack, editor: &Rc<Editor>, path: String) {
    if let Ok(page) = std::env::var("OPENSTEEL_PAGE") {
        stack.set_visible_child_name(&page);
    }
    if let Ok(text) = std::env::var("OPENSTEEL_TYPE") {
        let leds: Vec<u8> = text
            .chars()
            .filter_map(|c| editor.keys().iter().find(|k| k.label.eq_ignore_ascii_case(&c.to_string())).map(|k| k.led))
            .collect();
        let next = std::cell::Cell::new(0);
        glib::timeout_add_local(std::time::Duration::from_millis(110), glib::clone!(
            #[strong] editor,
            move || {
                let Some(&led) = leds.get(next.get()) else { return glib::ControlFlow::Break };
                editor.key_pressed(led);
                next.set(next.get() + 1);
                glib::ControlFlow::Continue
            }
        ));
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

fn save_session(editor: &Editor) {
    if let Err(e) = Profile::capture(&editor.document()).save(&profile::session_path()) {
        eprintln!("Couldn't save the session: {e}");
    }
}

/// Quitting has to be possible while the window is hidden, so it is an app action:
/// from the menu, Ctrl+Q, the notification, or `open-steel-studio --quit`.
fn add_app_actions(app: &adw::Application, window: &adw::ApplicationWindow, editor: &Rc<Editor>) {
    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate(glib::clone!(
        #[strong] editor,
        #[weak] app,
        move |_, _| {
            save_session(&editor);
            app.quit();
        }
    ));
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    let show = gio::SimpleAction::new("show", None);
    show.connect_activate(glib::clone!(#[weak] window, move |_, _| window.present()));
    app.add_action(&show);
}

/// The first time the window is closed while sending, explain where the app went.
fn note_running_in_background(app: &adw::Application) {
    if Settings::load().background_note_shown {
        return;
    }
    // A launcher lets the user reopen the window, and lets GNOME show the notification.
    if let Err(e) = desktop::install_launcher() {
        eprintln!("Couldn't add OpenSteelStudio to the app menu: {e}");
    }
    let notification = gio::Notification::new("OpenSteelStudio keeps running");
    notification.set_body(Some(
        "It keeps your keyboard's lighting and screen going. Open it from your app menu to make changes, or quit it from its menu.",
    ));
    notification.set_default_action("app.show");
    notification.add_button("Quit", "app.quit");
    app.send_notification(Some("background"), &notification);
    Settings::update(|s| s.background_note_shown = true);
}

fn add_actions(window: &adw::ApplicationWindow, editor: &Rc<Editor>, link: &Rc<Link>, stack: &adw::ViewStack, toasts: &adw::ToastOverlay) {
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

    action("restore-lighting", Box::new(glib::clone!(
        #[weak] toasts,
        #[strong] link,
        move || {
            let message = match link.restore_keyboard_lighting() {
                Ok(()) => "The keyboard is back on its own lighting".to_string(),
                Err(e) => format!("Couldn't reach the keyboard: {e}"),
            };
            toasts.add_toast(adw::Toast::new(&message));
        }
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
                    match Profile::load(&path).and_then(Profile::into_document) {
                        Ok(doc) => {
                            editor.load(doc);
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
                    match Profile::capture(&editor.document()).save(&path) {
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

    let autostart = gio::SimpleAction::new_stateful("autostart", None, &desktop::autostart_enabled().to_variant());
    autostart.connect_change_state(glib::clone!(
        #[weak] toasts,
        move |a, value| {
            let Some(on) = value.and_then(|v| v.get::<bool>()) else { return };
            let result = desktop::set_autostart(on).and_then(|()| if on { desktop::install_launcher() } else { Ok(()) });
            match result {
                Ok(()) => {
                    a.set_state(&on.to_variant());
                    if on {
                        toasts.add_toast(adw::Toast::new("OpenSteelStudio will start at login, without a window"));
                    }
                }
                Err(e) => toasts.add_toast(adw::Toast::new(&format!("Couldn't change the login setting: {e}"))),
            }
        }
    ));
    window.add_action(&autostart);

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
        match Profile::load(&path).and_then(Profile::into_document) {
            Ok(doc) => return Editor::new(doc),
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
    Editor::new(Document { region: Region::Ansi, lighting, screen, ..Default::default() })
}

/// Feeds keys typed while the window has focus into the typing effects. Key repeat is
/// ignored, like on the real keyboard where holding a key is a single press.
fn preview_typing(window: &adw::ApplicationWindow, editor: &Rc<Editor>) {
    let held = Rc::new(std::cell::RefCell::new(std::collections::HashSet::new()));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[strong] editor,
        #[strong] held,
        move |_, _, keycode, _| {
            // While the keyboard's own key reports are read, they already include these keys.
            if !editor.hardware_typing()
                && held.borrow_mut().insert(keycode)
                && let Some(led) = keymap::led_for_keycode(keycode, editor.region())
            {
                editor.key_pressed(led);
            }
            glib::Propagation::Proceed
        }
    ));
    keys.connect_key_released(glib::clone!(
        #[strong] held,
        move |_, _, keycode, _| {
            held.borrow_mut().remove(&keycode);
        }
    ));
    // Releases go unseen while another window has focus.
    window.connect_is_active_notify(glib::clone!(#[strong] held, move |_| held.borrow_mut().clear()));
    window.add_controller(keys);
}

/// Wires the "Send to keyboard" toggle and the status banner to the keyboard link.
fn connect_keyboard(window: &adw::ApplicationWindow, link: &Rc<Link>, send: &gtk::ToggleButton, banner: &adw::Banner, toasts: &adw::ToastOverlay) {
    send.connect_toggled(glib::clone!(
        #[strong] link,
        move |button| {
            let on = button.is_active();
            if on {
                button.add_css_class("suggested-action");
            } else {
                button.remove_css_class("suggested-action");
            }
            link.set_enabled(on);
            Settings::update(|s| s.send_to_keyboard = on);
        }
    ));

    link.connect_status(glib::clone!(
        #[weak] send,
        #[weak] banner,
        #[weak] toasts,
        #[strong] link,
        move |status| {
            if send.is_active() != link.is_enabled() {
                send.set_active(link.is_enabled());
            }
            send.set_tooltip_text(Some(&device::describe(status)));
            let (title, button) = match status {
                Status::Off => (None, None),
                Status::Connected => {
                    toasts.add_toast(adw::Toast::builder().title("Connected to your Apex 5").timeout(2).build());
                    (None, None)
                }
                Status::NotFound => (Some("Looking for your Apex 5. Plug it in and it connects by itself.".to_string()), None),
                Status::NoPermission => (Some("OpenSteelStudio isn't allowed to use the keyboard yet.".to_string()), Some("How to fix")),
                Status::Unexpected => (
                    Some("This keyboard describes itself differently from the Apex 5 this app was checked against, so nothing is sent to keep it safe.".to_string()),
                    None,
                ),
                Status::Failed(e) => (Some(format!("Couldn't talk to the keyboard ({e}). Trying again…")), None),
            };
            banner.set_title(title.as_deref().unwrap_or_default());
            banner.set_button_label(button);
            banner.set_revealed(title.is_some());
        }
    ));

    banner.connect_button_clicked(glib::clone!(#[weak] window, move |_| show_access_help(&window)));
}

fn access_commands() -> String {
    format!(
        "sudo tee /etc/udev/rules.d/70-open-steel-studio.rules > /dev/null << 'EOF'\n{ACCESS_RULES}EOF\n\
         sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw\n"
    )
}

fn show_access_help(window: &adw::ApplicationWindow) {
    let dialog = adw::AlertDialog::builder()
        .heading("Allow access to the keyboard")
        .body(
            "Linux only lets programs control the keyboard with permission. Run these commands in a terminal once, \
             and OpenSteelStudio connects by itself. They only change who may open the keyboard's device files.",
        )
        .build();
    let commands = gtk::Label::builder()
        .label(access_commands())
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .css_classes(["monospace", "caption"])
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(10)
        .margin_end(10)
        .build();
    let frame = gtk::Box::builder().css_classes(["card"]).build();
    frame.append(&commands);
    dialog.set_extra_child(Some(&frame));
    dialog.add_responses(&[("close", "Close"), ("copy", "Copy commands")]);
    dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("copy"));
    dialog.connect_response(Some("copy"), |dialog, _| dialog.clipboard().set_text(&access_commands()));
    dialog.present(Some(window));
}
