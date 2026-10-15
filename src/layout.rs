//! Physical key layout of the SteelSeries Apex 5 (full size, ANSI and ISO).
//!
//! Positions are in key units (1u = one standard keycap). Each key carries the
//! LED id the firmware uses for it: the key's USB HID usage code, the same ids
//! OpenRGB uses for the Apex series.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    #[default]
    Ansi,
    Iso,
}

/// Width and height of the key area in key units.
pub const WIDTH: f64 = 22.5;
pub const HEIGHT: f64 = 6.5;

/// Where the OLED sits on the case (x, y, w, h), in key units. The panel is 128×40.
pub const OLED: (f64, f64, f64, f64) = (18.5, 0.05, 2.88, 0.9);

#[derive(Clone, Debug)]
pub struct Key {
    pub led: u8,
    pub label: &'static str,
    pub name: &'static str,
    /// Outline polygon in key units, clockwise.
    pub outline: Vec<(f64, f64)>,
    pub label_at: (f64, f64),
}

impl Key {
    /// (x0, y0, x1, y1)
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        self.outline.iter().fold(
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        )
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        // Even-odd ray casting; enough for the rectangles and the ISO Enter L-shape.
        let pts = &self.outline;
        let mut inside = false;
        let mut j = pts.len() - 1;
        for i in 0..pts.len() {
            let (xi, yi) = pts[i];
            let (xj, yj) = pts[j];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    pub fn intersects(&self, (rx0, ry0, rx1, ry1): (f64, f64, f64, f64)) -> bool {
        let (x0, y0, x1, y1) = self.bounds();
        x0 < rx1 && rx0 < x1 && y0 < ry1 && ry0 < y1
    }
}

#[derive(Default)]
struct Builder {
    keys: Vec<Key>,
}

impl Builder {
    #[allow(clippy::too_many_arguments)]
    fn key(&mut self, led: u8, label: &'static str, name: &'static str, x: f64, y: f64, w: f64, h: f64) {
        self.keys.push(Key {
            led,
            label,
            name,
            outline: vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)],
            label_at: (x + w / 2.0, y + h / 2.0),
        });
    }

    /// A run of 1u keys whose name is their label.
    fn row(&mut self, x: f64, y: f64, keys: &[(u8, &'static str)]) {
        for (i, &(led, label)) in keys.iter().enumerate() {
            self.key(led, label, label, x + i as f64, y, 1.0, 1.0);
        }
    }
}

pub fn keys(region: Region) -> Vec<Key> {
    let iso = region == Region::Iso;
    let mut b = Builder::default();

    // Function row
    b.key(0x29, "Esc", "Escape", 0.0, 0.0, 1.0, 1.0);
    b.row(2.0, 0.0, &[(0x3A, "F1"), (0x3B, "F2"), (0x3C, "F3"), (0x3D, "F4")]);
    b.row(6.5, 0.0, &[(0x3E, "F5"), (0x3F, "F6"), (0x40, "F7"), (0x41, "F8")]);
    b.row(11.0, 0.0, &[(0x42, "F9"), (0x43, "F10"), (0x44, "F11"), (0x45, "F12")]);
    b.key(0x46, "PrtSc", "Print Screen", 15.25, 0.0, 1.0, 1.0);
    b.key(0x47, "ScrLk", "Scroll Lock", 16.25, 0.0, 1.0, 1.0);
    b.key(0x48, "Pause", "Pause / Break", 17.25, 0.0, 1.0, 1.0);

    // Number row
    let y = 1.5;
    b.row(0.0, y, &[
        (0x35, "`"), (0x1E, "1"), (0x1F, "2"), (0x20, "3"), (0x21, "4"), (0x22, "5"), (0x23, "6"),
        (0x24, "7"), (0x25, "8"), (0x26, "9"), (0x27, "0"), (0x2D, "-"), (0x2E, "="),
    ]);
    b.key(0x2A, "Backspace", "Backspace", 13.0, y, 2.0, 1.0);
    b.key(0x49, "Ins", "Insert", 15.25, y, 1.0, 1.0);
    b.key(0x4A, "Home", "Home", 16.25, y, 1.0, 1.0);
    b.key(0x4B, "PgUp", "Page Up", 17.25, y, 1.0, 1.0);
    b.key(0x53, "Num", "Num Lock", 18.5, y, 1.0, 1.0);
    b.row(19.5, y, &[(0x54, "/"), (0x55, "*"), (0x56, "-")]);

    // Top letter row
    let y = 2.5;
    b.key(0x2B, "Tab", "Tab", 0.0, y, 1.5, 1.0);
    b.row(1.5, y, &[
        (0x14, "Q"), (0x1A, "W"), (0x08, "E"), (0x15, "R"), (0x17, "T"), (0x1C, "Y"),
        (0x18, "U"), (0x0C, "I"), (0x12, "O"), (0x13, "P"), (0x2F, "["), (0x30, "]"),
    ]);
    if iso {
        // ISO Enter spans two rows: 1.5u wide on top, 1.25u below.
        b.keys.push(Key {
            led: 0x28,
            label: "Enter",
            name: "Enter",
            outline: vec![(13.5, 2.5), (15.0, 2.5), (15.0, 4.5), (13.75, 4.5), (13.75, 3.5), (13.5, 3.5)],
            label_at: (14.3, 3.0),
        });
    } else {
        b.key(0x31, "\\", "Backslash", 13.5, y, 1.5, 1.0);
    }
    b.key(0x4C, "Del", "Delete", 15.25, y, 1.0, 1.0);
    b.key(0x4D, "End", "End", 16.25, y, 1.0, 1.0);
    b.key(0x4E, "PgDn", "Page Down", 17.25, y, 1.0, 1.0);
    b.row(18.5, y, &[(0x5F, "7"), (0x60, "8"), (0x61, "9")]);
    b.key(0x57, "+", "Numpad +", 21.5, y, 1.0, 2.0);

    // Home row
    let y = 3.5;
    b.key(0x39, "Caps", "Caps Lock", 0.0, y, 1.75, 1.0);
    b.row(1.75, y, &[
        (0x04, "A"), (0x16, "S"), (0x07, "D"), (0x09, "F"), (0x0A, "G"), (0x0B, "H"),
        (0x0D, "J"), (0x0E, "K"), (0x0F, "L"), (0x33, ";"), (0x34, "'"),
    ]);
    if iso {
        b.key(0x32, "#", "Hash (ISO)", 12.75, y, 1.0, 1.0);
    } else {
        b.key(0x28, "Enter", "Enter", 12.75, y, 2.25, 1.0);
    }
    b.row(18.5, y, &[(0x5C, "4"), (0x5D, "5"), (0x5E, "6")]);

    // Bottom letter row
    let y = 4.5;
    if iso {
        b.key(0xE1, "Shift", "Left Shift", 0.0, y, 1.25, 1.0);
        b.key(0x64, "\\", "Backslash (ISO)", 1.25, y, 1.0, 1.0);
    } else {
        b.key(0xE1, "Shift", "Left Shift", 0.0, y, 2.25, 1.0);
    }
    b.row(2.25, y, &[
        (0x1D, "Z"), (0x1B, "X"), (0x06, "C"), (0x19, "V"), (0x05, "B"),
        (0x11, "N"), (0x10, "M"), (0x36, ","), (0x37, "."), (0x38, "/"),
    ]);
    b.key(0xE5, "Shift", "Right Shift", 12.25, y, 2.75, 1.0);
    b.key(0x52, "↑", "Up", 16.25, y, 1.0, 1.0);
    b.row(18.5, y, &[(0x59, "1"), (0x5A, "2"), (0x5B, "3")]);
    b.key(0x58, "Enter", "Numpad Enter", 21.5, y, 1.0, 2.0);

    // Modifier row
    let y = 5.5;
    b.key(0xE0, "Ctrl", "Left Ctrl", 0.0, y, 1.25, 1.0);
    b.key(0xE3, "Win", "Left Super", 1.25, y, 1.25, 1.0);
    b.key(0xE2, "Alt", "Left Alt", 2.5, y, 1.25, 1.0);
    b.key(0x2C, "", "Space", 3.75, y, 6.25, 1.0);
    b.key(0xE6, "Alt", "Right Alt", 10.0, y, 1.25, 1.0);
    b.key(0xE7, "SS", "SteelSeries key", 11.25, y, 1.25, 1.0);
    b.key(0xF0, "Menu", "Menu", 12.5, y, 1.25, 1.0);
    b.key(0xE4, "Ctrl", "Right Ctrl", 13.75, y, 1.25, 1.0);
    b.key(0x50, "←", "Left", 15.25, y, 1.0, 1.0);
    b.key(0x51, "↓", "Down", 16.25, y, 1.0, 1.0);
    b.key(0x4F, "→", "Right", 17.25, y, 1.0, 1.0);
    b.key(0x62, "0", "Numpad 0", 18.5, y, 2.0, 1.0);
    b.key(0x63, ".", "Numpad .", 20.5, y, 1.0, 1.0);

    b.keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn key_counts() {
        assert_eq!(keys(Region::Ansi).len(), 104);
        assert_eq!(keys(Region::Iso).len(), 105);
    }

    #[test]
    fn leds_are_unique_and_in_bounds() {
        for region in [Region::Ansi, Region::Iso] {
            let keys = keys(region);
            let leds: HashSet<u8> = keys.iter().map(|k| k.led).collect();
            assert_eq!(leds.len(), keys.len(), "duplicate LED id in {region:?}");
            for k in &keys {
                let (x0, y0, x1, y1) = k.bounds();
                assert!(x0 >= 0.0 && y0 >= 0.0 && x1 <= WIDTH && y1 <= HEIGHT, "{} out of bounds", k.name);
            }
        }
    }

    #[test]
    fn keys_do_not_overlap() {
        for region in [Region::Ansi, Region::Iso] {
            let keys = keys(region);
            // Sample a grid of points; no point may be inside two keys.
            for yi in 0..130 {
                for xi in 0..450 {
                    let (x, y) = (xi as f64 * 0.05 + 0.01, yi as f64 * 0.05 + 0.01);
                    let hits = keys.iter().filter(|k| k.contains(x, y)).count();
                    assert!(hits <= 1, "overlap at ({x}, {y}) in {region:?}");
                }
            }
        }
    }

    #[test]
    fn iso_enter_shape() {
        let keys = keys(Region::Iso);
        let enter = keys.iter().find(|k| k.led == 0x28).unwrap();
        assert!(enter.contains(13.6, 3.0)); // top part
        assert!(enter.contains(14.5, 4.0)); // bottom part
        assert!(!enter.contains(13.6, 4.0)); // the notch belongs to '#'
    }
}
