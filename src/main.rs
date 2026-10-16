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
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "io.github.Spider_Frog.OpenSteelStudio";

fn main() -> glib::ExitCode {
    // Command-line tools, handled before GTK starts.
    match std::env::args().nth(1).as_deref() {
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
    // Development screenshots run as a separate instance that never talks to a running copy.
    #[cfg(debug_assertions)]
    if std::env::var_os("OPENSTEEL_SCREENSHOT").is_some() {
        app.set_flags(gio::ApplicationFlags::NON_UNIQUE);
    }
    app.connect_startup(|_| ui::load_css());
    app.connect_activate(window::build);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    app.set_accels_for_action("win.save", &["<Control>s"]);

    if quit {
        // Stops the running copy, which may have no window to close.
        return match running_instance() {
            Some(bus) => match bus.call_sync(
                Some(APP_ID),
                &object_path(),
                "org.gtk.Actions",
                "Activate",
                Some(&("quit", Vec::<glib::Variant>::new(), std::collections::HashMap::<String, glib::Variant>::new()).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
            ) {
                Ok(_) => glib::ExitCode::SUCCESS,
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
    if background {
        if running_instance().is_some() {
            return glib::ExitCode::SUCCESS; // already running; don't pop up its window
        }
        window::start_hidden();
    }
    app.run_with_args(&args)
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
