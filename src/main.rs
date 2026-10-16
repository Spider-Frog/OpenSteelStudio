// SPDX-License-Identifier: GPL-3.0-or-later

//! OpenSteelStudio: lighting and OLED screen editor for SteelSeries keyboards.

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
use gtk::glib;

const APP_ID: &str = "io.github.Spider_Frog.OpenSteelStudio";

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

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| ui::load_css());
    app.connect_activate(window::build);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    app.set_accels_for_action("win.save", &["<Control>s"]);
    app.run()
}
