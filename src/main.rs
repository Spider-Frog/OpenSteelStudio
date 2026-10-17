// SPDX-License-Identifier: GPL-3.0-or-later

//! OpenSteelStudio: lighting and OLED screen editor for SteelSeries keyboards.

mod desktop;
mod device;
mod draw;
mod editor;
mod effects;
mod engine;
mod keyboard_view;
mod keymap;
mod layout;
mod lighting_page;
mod model;
mod oled_art;
mod profile;
mod screen_fx;
mod screen_page;
mod ui;
mod volume;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "io.github.Spider_Frog.OpenSteelStudio";

fn main() -> glib::ExitCode {
    // Command-line tools, handled before GTK starts.
    match std::env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("OpenSteelStudio {}", env!("CARGO_PKG_VERSION"));
            return glib::ExitCode::SUCCESS;
        }
        Some("--probe") => {
            // Read-only: finds the keyboard and checks its descriptors, sends nothing.
            println!("{}", device::probe());
            return glib::ExitCode::SUCCESS;
        }
        Some("--restore-lighting") => {
            return match device::send_lighting_reset() {
                Ok(()) => {
                    println!("The keyboard is back on its own lighting.");
                    glib::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    glib::ExitCode::FAILURE
                }
            };
        }
        _ => {}
    }

    let mut args: Vec<String> = std::env::args().collect();
    let background = args.iter().any(|a| a == desktop::BACKGROUND_FLAG);
    let quit = args.iter().any(|a| a == "--quit");
    args.retain(|a| a != desktop::BACKGROUND_FLAG && a != "--quit");

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| ui::load_css());
    app.connect_activate(window::build);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    app.set_accels_for_action("win.save", &["<Control>s"]);
    // Tells a copy started later which build this is (see `replace_other_build`).
    let build = gio::SimpleAction::new_stateful("build", None, &build_id().to_variant());
    build.set_enabled(false);
    app.add_action(&build);

    // Development screenshots run as a separate instance that never talks to a running copy.
    #[cfg(debug_assertions)]
    if std::env::var_os("OPENSTEEL_SCREENSHOT").is_some() {
        app.set_flags(gio::ApplicationFlags::NON_UNIQUE);
        return app.run_with_args(&args);
    }

    if quit {
        return match running_instance() {
            Some(bus) => match stop(&bus) {
                Ok(()) => glib::ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Couldn't stop OpenSteelStudio: {e}");
                    glib::ExitCode::FAILURE
                }
            },
            None => {
                println!("OpenSteelStudio isn't running.");
                glib::ExitCode::SUCCESS
            }
        };
    }
    replace_other_build();
    if background {
        if running_instance().is_some() {
            return glib::ExitCode::SUCCESS; // already running; don't pop up its window
        }
        window::start_hidden();
    }
    app.run_with_args(&args)
}

/// Which build this is: its version and where it lives. Two copies of the same build share
/// a window; any other build (another version, or a copy elsewhere, like an AppImage next to
/// an installed package) takes over instead.
fn build_id() -> String {
    let program = desktop::program().map(|p| p.display().to_string()).unwrap_or_default();
    format!("{} {program}", env!("CARGO_PKG_VERSION"))
}

/// Stops a running copy of a different build, so this one runs instead of handing over to
/// it. The stopped copy saves its session first, so nothing is lost.
fn replace_other_build() {
    let Some(bus) = running_instance() else { return };
    if running_build(&bus).is_some_and(|other| other == build_id()) {
        return; // the same build: GApplication shows its window
    }
    if let Err(e) = stop(&bus) {
        return eprintln!("Couldn't stop the OpenSteelStudio that is running: {e}");
    }
    for _ in 0..50 {
        if running_instance().is_none() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("The OpenSteelStudio that was running didn't stop in time");
}

/// The build of the running copy. Copies from before 0.1.3 don't say, which counts as different.
fn running_build(bus: &gio::DBusConnection) -> Option<String> {
    let reply = bus
        .call_sync(
            Some(APP_ID),
            &object_path(),
            "org.gtk.Actions",
            "Describe",
            Some(&("build",).to_variant()),
            glib::VariantTy::new("((bgav))").ok(),
            gio::DBusCallFlags::NONE,
            -1,
            gio::Cancellable::NONE,
        )
        .ok()?;
    // The reply holds one (enabled, parameter type, [state]) structure.
    let state = reply.child_value(0).child_value(2);
    (state.n_children() > 0).then(|| state.child_value(0).as_variant()?.str().map(str::to_owned))?
}

/// Asks the running copy to quit, which works even while it has no window.
fn stop(bus: &gio::DBusConnection) -> Result<(), glib::Error> {
    let no_parameters = Vec::<glib::Variant>::new();
    let no_platform_data = std::collections::HashMap::<String, glib::Variant>::new();
    bus.call_sync(
        Some(APP_ID),
        &object_path(),
        "org.gtk.Actions",
        "Activate",
        Some(&("quit", no_parameters, no_platform_data).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
    )
    .map(|_| ())
}
/// The session bus, if a copy of the app is already running on it.
fn running_instance() -> Option<gio::DBusConnection> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
    let reply = bus
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(APP_ID,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            -1,
            gio::Cancellable::NONE,
        )
        .ok()?;
    reply.get::<(bool,)>().is_some_and(|(owned,)| owned).then_some(bus)
}

/// Where GApplication exports its actions: the app id as a path, like /io/github/….
fn object_path() -> String {
    format!("/{}", APP_ID.replace('.', "/").replace('-', "_"))
}
