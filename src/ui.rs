// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared widget helpers and the app's stylesheet.

use adw::prelude::*;
use gtk::gdk;

use crate::lighting_page::SWATCHES;

/// A titled card. Returns the card and the box to append its content to.
pub fn card(title: &str, subtitle: &str) -> (gtk::Box, gtk::Box) {
    let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["card"]).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(14)
        .margin_bottom(14)
        .margin_start(14)
        .margin_end(14)
        .build();
    content.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["heading"]).build());
    if !subtitle.is_empty() {
        content.append(
            &gtk::Label::builder()
                .label(subtitle)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["dim-label", "caption"])
                .build(),
        );
    }
    outer.append(&content);
    (outer, content)
}

pub fn icon_button_content(icon: &str, label: &str) -> adw::ButtonContent {
    adw::ButtonContent::builder().icon_name(icon).label(label).use_underline(true).build()
}

/// Shows a toast in the window that contains `widget`.
pub fn toast(widget: &impl IsA<gtk::Widget>, message: &str) {
    if let Some(overlay) = widget.ancestor(adw::ToastOverlay::static_type()).and_downcast::<adw::ToastOverlay>() {
        overlay.add_toast(adw::Toast::builder().title(message).timeout(3).build());
    }
}

pub fn load_css() {
    let mut css = String::from(
        "button.swatch { min-width: 26px; min-height: 26px; padding: 0; border-radius: 8px; \
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.18); }\n\
         .oled-frame { background: black; border-radius: 12px; }\n",
    );
    for (i, c) in SWATCHES.iter().enumerate() {
        css.push_str(&format!("button.swatch-{i} {{ background: {}; }}\n", c.to_hex()));
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
