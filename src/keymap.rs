// SPDX-License-Identifier: GPL-3.0-or-later

//! Maps Linux key codes to the keyboard's LED ids (USB HID usage codes).
//!
//! Used to preview typing effects with keys pressed while the app has focus.

use crate::layout::Region;

/// GDK hardware keycodes on Linux are evdev codes plus 8.
pub fn led_for_keycode(keycode: u32, region: Region) -> Option<u8> {
    led_for_evdev(keycode.checked_sub(8)?, region)
}

pub fn led_for_evdev(code: u32, region: Region) -> Option<u8> {
    Some(match code {
        1 => 0x29,                                // Esc
        2..=10 => 0x1E + (code - 2) as u8,        // 1..9
        11 => 0x27,                               // 0
        12 => 0x2D,                               // -
        13 => 0x2E,                               // =
        14 => 0x2A,                               // Backspace
        15 => 0x2B,                               // Tab
        16 => 0x14, 17 => 0x1A, 18 => 0x08, 19 => 0x15, 20 => 0x17, // Q W E R T
        21 => 0x1C, 22 => 0x18, 23 => 0x0C, 24 => 0x12, 25 => 0x13, // Y U I O P
        26 => 0x2F,                               // [
        27 => 0x30,                               // ]
        28 => 0x28,                               // Enter
        29 => 0xE0,                               // Left Ctrl
        30 => 0x04, 31 => 0x16, 32 => 0x07, 33 => 0x09, 34 => 0x0A, // A S D F G
        35 => 0x0B, 36 => 0x0D, 37 => 0x0E, 38 => 0x0F,             // H J K L
        39 => 0x33,                               // ;
        40 => 0x34,                               // '
        41 => 0x35,                               // `
        42 => 0xE1,                               // Left Shift
        // Linux reports the ANSI backslash and the ISO '#' key as the same code.
        43 => if region == Region::Iso { 0x32 } else { 0x31 },
        44 => 0x1D, 45 => 0x1B, 46 => 0x06, 47 => 0x19, 48 => 0x05, // Z X C V B
        49 => 0x11, 50 => 0x10,                                     // N M
        51 => 0x36,                               // ,
        52 => 0x37,                               // .
        53 => 0x38,                               // /
        54 => 0xE5,                               // Right Shift
        55 => 0x55,                               // Numpad *
        56 => 0xE2,                               // Left Alt
        57 => 0x2C,                               // Space
        58 => 0x39,                               // Caps Lock
        59..=68 => 0x3A + (code - 59) as u8,      // F1..F10
        69 => 0x53,                               // Num Lock
        70 => 0x47,                               // Scroll Lock
        71 => 0x5F, 72 => 0x60, 73 => 0x61,      // Numpad 7 8 9
        74 => 0x56,                               // Numpad -
        75 => 0x5C, 76 => 0x5D, 77 => 0x5E,      // Numpad 4 5 6
        78 => 0x57,                               // Numpad +
        79 => 0x59, 80 => 0x5A, 81 => 0x5B,      // Numpad 1 2 3
        82 => 0x62,                               // Numpad 0
        83 => 0x63,                               // Numpad .
        86 => 0x64,                               // ISO backslash
        87 => 0x44,                               // F11
        88 => 0x45,                               // F12
        96 => 0x58,                               // Numpad Enter
        97 => 0xE4,                               // Right Ctrl
        98 => 0x54,                               // Numpad /
        99 => 0x46,                               // Print Screen
        100 => 0xE6,                              // Right Alt
        102 => 0x4A,                              // Home
        103 => 0x52,                              // Up
        104 => 0x4B,                              // Page Up
        105 => 0x50,                              // Left
        106 => 0x4F,                              // Right
        107 => 0x4D,                              // End
        108 => 0x51,                              // Down
        109 => 0x4E,                              // Page Down
        110 => 0x49,                              // Insert
        111 => 0x4C,                              // Delete
        119 => 0x48,                              // Pause
        125 => 0xE3,                              // Left Super
        126 => 0xE7,                              // Right Super (SteelSeries key position)
        127 => 0xF0,                              // Menu
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;

    #[test]
    fn every_key_is_reachable() {
        for region in [Region::Ansi, Region::Iso] {
            let mapped: std::collections::HashSet<u8> = (0..256).filter_map(|c| led_for_evdev(c, region)).collect();
            for key in layout::keys(region) {
                assert!(mapped.contains(&key.led), "{} ({:#04x}) has no key code in {region:?}", key.name, key.led);
            }
        }
    }

    #[test]
    fn letters() {
        assert_eq!(led_for_keycode(30 + 8, Region::Ansi), Some(0x04)); // A
        assert_eq!(led_for_keycode(17 + 8, Region::Ansi), Some(0x1A)); // W
        assert_eq!(led_for_keycode(3, Region::Ansi), None);
    }
}
