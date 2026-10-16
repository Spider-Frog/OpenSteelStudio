// SPDX-License-Identifier: GPL-3.0-or-later

//! SteelSeries Apex 5 (USB 1038:161c): packet formats and key report parsing.
//!
//! The per-key lighting and lighting-reset commands follow OpenRGB's driver for
//! first-generation Apex keyboards; the screen frame layout follows apex-tux. All three
//! change temporary state only. Nothing here writes the keyboard's flash or firmware.

use crate::model::{Bitmap, Rgb, SCREEN_H, SCREEN_W};

pub const VENDOR: u16 = 0x1038;
pub const PRODUCT: u16 = 0x161C;

/// Vendor interface: lighting and screen (feature reports), lighting reset (output report).
pub const CONTROL_INTERFACE: u8 = 1;
/// Standard keyboard interfaces: boot protocol (six keys) and full (one bit per key).
pub const BOOT_KEYS_INTERFACE: u8 = 0;
pub const ALL_KEYS_INTERFACE: u8 = 2;

/// HID report descriptors as read from an Apex 5. Nothing is sent to a keyboard whose
/// descriptors differ, because then these packet formats can't be trusted.
pub const CONTROL_DESCRIPTOR: &[u8] = &[
    0x06, 0xc0, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x06, 0xc1, 0xff, 0x15, 0x00, //
    0x26, 0xff, 0x00, 0x75, 0x08, 0x09, 0xf0, 0x95, 0x40, 0x81, 0x02, 0x09, //
    0xf1, 0x95, 0x40, 0x91, 0x02, 0x09, 0xf2, 0x96, 0x82, 0x02, 0xb1, 0x02, //
    0xc0,
];
pub const BOOT_KEYS_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, //
    0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x75, 0x08, //
    0x95, 0x01, 0x81, 0x01, 0x05, 0x07, 0x19, 0x00, 0x2a, 0xff, 0x00, 0x15, //
    0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0x05, 0x08, //
    0x19, 0x01, 0x29, 0x03, 0x25, 0x01, 0x75, 0x01, 0x95, 0x03, 0x91, 0x02, //
    0x95, 0x05, 0x91, 0x01, 0xc0,
];
pub const ALL_KEYS_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0x00, 0x2a, 0xff, //
    0x00, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x96, 0x00, 0x01, 0x81, 0x02, //
    0x05, 0x08, 0x19, 0x01, 0x29, 0x03, 0x25, 0x01, 0x75, 0x01, 0x95, 0x03, //
    0x91, 0x02, 0x95, 0x05, 0x91, 0x01, 0xc0,
];

/// Feature report as the descriptor declares it: 642 bytes, plus the leading report
/// id 0 that Linux expects and strips (the keyboard doesn't use report ids).
pub const FEATURE_LEN: usize = 1 + 642;
/// Output report: 64 bytes plus the report id.
pub const OUTPUT_LEN: usize = 1 + 64;

pub const CMD_LIGHTING: u8 = 0x3A;
pub const CMD_LIGHTING_RESET: u8 = 0x3B;
pub const CMD_SCREEN: u8 = 0x61;

/// How many keys one lighting packet can carry: 4 bytes each after the 3-byte header.
pub const MAX_KEYS: usize = (FEATURE_LEN - 3) / 4;

/// Sets the color of each listed LED (LED ids are the keys' HID usage codes).
pub fn lighting_packet(colors: &[(u8, Rgb)]) -> [u8; FEATURE_LEN] {
    let mut packet = [0u8; FEATURE_LEN];
    let count = colors.len().min(MAX_KEYS);
    packet[1] = CMD_LIGHTING;
    packet[2] = count as u8;
    for (i, &(led, c)) in colors.iter().take(count).enumerate() {
        packet[3 + i * 4..7 + i * 4].copy_from_slice(&[led, c.r, c.g, c.b]);
    }
    packet
}

/// Shows a 128×40 image: rows top to bottom, 16 bytes per row, leftmost pixel in the high bit.
pub fn screen_packet(bitmap: &Bitmap) -> [u8; FEATURE_LEN] {
    let mut packet = [0u8; FEATURE_LEN];
    packet[1] = CMD_SCREEN;
    for y in 0..SCREEN_H {
        for x in 0..SCREEN_W {
            if bitmap.get(x as i32, y as i32) {
                let bit = y * SCREEN_W + x;
                packet[2 + bit / 8] |= 0x80 >> (bit % 8);
            }
        }
    }
    packet
}

/// Hands the lighting back to the keyboard's own (on-board) lighting.
pub fn lighting_reset_packet() -> [u8; OUTPUT_LEN] {
    let mut packet = [0u8; OUTPUT_LEN];
    packet[1] = CMD_LIGHTING_RESET;
    packet
}

/// Keys held in a boot keyboard report: [modifier bits, reserved, six key codes].
pub fn boot_keys(report: &[u8]) -> Vec<u8> {
    let Some((&modifiers, rest)) = report.split_first() else { return vec![] };
    let mut keys: Vec<u8> = (0..8).filter(|bit| modifiers & (1 << bit) != 0).map(|bit| 0xE0 + bit).collect();
    // Codes 1-3 signal errors such as too many keys at once, not keys.
    keys.extend(rest.iter().skip(1).copied().filter(|&code| code >= 0x04));
    keys
}

/// Keys held in a full keyboard report: one bit per key code, lowest bit first.
pub fn all_keys(report: &[u8]) -> Vec<u8> {
    (0x04..=0xFFu16)
        .map(|code| code as u8)
        .filter(|&code| report.get(code as usize / 8).is_some_and(|byte| byte & (1 << (code % 8)) != 0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lighting_packet_layout() {
        let p = lighting_packet(&[(0x04, Rgb::new(1, 2, 3)), (0xE0, Rgb::new(250, 251, 252))]);
        assert_eq!(p.len(), 643);
        assert_eq!(&p[..11], &[0x00, 0x3A, 2, 0x04, 1, 2, 3, 0xE0, 250, 251, 252]);
        assert!(p[11..].iter().all(|&b| b == 0));
    }

    #[test]
    fn lighting_packet_never_overflows() {
        let many: Vec<(u8, Rgb)> = (0..=255).map(|led| (led as u8, Rgb::new(9, 9, 9))).collect();
        let p = lighting_packet(&many);
        assert_eq!(p[2] as usize, MAX_KEYS);
        assert_eq!(MAX_KEYS, 160);
    }

    #[test]
    fn screen_packet_layout() {
        let mut bitmap = Bitmap::default();
        bitmap.set(0, 0, true); // first byte, high bit
        bitmap.set(9, 0, true); // second byte, bit 6
        bitmap.set(127, 39, true); // last pixel
        let p = screen_packet(&bitmap);
        assert_eq!(p.len(), 643);
        assert_eq!(&p[..4], &[0x00, 0x61, 0x80, 0x40]);
        assert_eq!(p[2 + 639], 0x01);
        assert_eq!(p[642], 0x00, "trailing byte stays zero");
        assert_eq!(p.iter().map(|b| b.count_ones()).sum::<u32>(), 3 + 0x61u8.count_ones());
    }

    #[test]
    fn reset_packet_layout() {
        let p = lighting_reset_packet();
        assert_eq!(p.len(), 65);
        assert_eq!(&p[..2], &[0x00, 0x3B]);
        assert!(p[2..].iter().all(|&b| b == 0));
    }

    #[test]
    fn packet_sizes_match_the_descriptor() {
        // Feature report count 0x0282 = 642 and output report count 0x40 = 64.
        assert!(CONTROL_DESCRIPTOR.windows(3).any(|w| w == [0x96, 0x82, 0x02]));
        assert!(CONTROL_DESCRIPTOR.windows(2).any(|w| w == [0x95, 0x40]));
        assert_eq!(FEATURE_LEN - 1, 0x0282);
        assert_eq!(OUTPUT_LEN - 1, 0x40);
    }

    #[test]
    fn parses_boot_reports() {
        // Left Shift + Right Alt held, keys A and S, one empty slot, one error code.
        let report = [0b0100_0010, 0, 0x04, 0x16, 0x00, 0x01, 0x00, 0x00];
        assert_eq!(boot_keys(&report), vec![0xE1, 0xE6, 0x04, 0x16]);
        assert!(boot_keys(&[]).is_empty());
    }

    #[test]
    fn parses_bitmap_reports() {
        let mut report = [0u8; 32];
        report[0] |= 0b0001_0000; // code 0x04: A
        report[0xE0 / 8] |= 0b0000_0001; // code 0xE0: Left Ctrl
        report[0] |= 0b0000_0010; // code 1 is not a key
        assert_eq!(all_keys(&report), vec![0x04, 0xE0]);
        assert!(all_keys(&report[..4]).contains(&0x04), "short reports are fine");
    }
}
