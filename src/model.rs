// SPDX-License-Identifier: GPL-3.0-or-later

//! Plain data: colors, the per-key lighting state and the 128×40 screen bitmap.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb::new(0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Accepts `#rrggbb`, `rrggbb`, `#rgb` and `rgb`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let expanded: String = match s.len() {
            3 => s.chars().flat_map(|c| [c, c]).collect(),
            6 => s.to_string(),
            _ => return None,
        };
        let v = u32::from_str_radix(&expanded, 16).ok()?;
        Some(Self::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// `h` in degrees, `s` and `v` in 0..=1.
    pub fn from_hsv(h: f64, s: f64, v: f64) -> Self {
        let h = h.rem_euclid(360.0) / 60.0;
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let f = |u: f64| ((u + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Self::new(f(r), f(g), f(b))
    }

    /// Perceived brightness in 0..=1.
    pub fn luma(self) -> f64 {
        (0.299 * self.r as f64 + 0.587 * self.g as f64 + 0.114 * self.b as f64) / 255.0
    }

    /// Linear blend from `self` (t = 0) to `other` (t = 1).
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let f = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
        Rgb::new(f(self.r, other.r), f(self.g, other.g), f(self.b, other.b))
    }

    pub fn to_f64(self) -> (f64, f64, f64) {
        (self.r as f64 / 255.0, self.g as f64 / 255.0, self.b as f64 / 255.0)
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid color {s:?}")))
    }
}

/// A color for every LED id. The firmware identifies LEDs by the key's HID
/// usage code, so every id fits in a byte.
#[derive(Clone, PartialEq)]
pub struct Lighting {
    colors: [Rgb; 256],
}

impl Default for Lighting {
    fn default() -> Self {
        Self { colors: [Rgb::BLACK; 256] }
    }
}

impl Lighting {
    pub fn get(&self, led: u8) -> Rgb {
        self.colors[led as usize]
    }

    pub fn set(&mut self, led: u8, color: Rgb) {
        self.colors[led as usize] = color;
    }
}

pub const SCREEN_W: usize = 128;
pub const SCREEN_H: usize = 40;

/// The monochrome OLED image, row-major, `true` = lit pixel.
#[derive(Clone, PartialEq, Debug)]
pub struct Bitmap {
    px: Vec<bool>,
}

impl Default for Bitmap {
    fn default() -> Self {
        Self { px: vec![false; SCREEN_W * SCREEN_H] }
    }
}

impl Bitmap {
    pub fn get(&self, x: i32, y: i32) -> bool {
        Self::index(x, y).is_some_and(|i| self.px[i])
    }

    pub fn set(&mut self, x: i32, y: i32, on: bool) {
        if let Some(i) = Self::index(x, y) {
            self.px[i] = on;
        }
    }

    fn index(x: i32, y: i32) -> Option<usize> {
        let inside = (0..SCREEN_W as i32).contains(&x) && (0..SCREEN_H as i32).contains(&y);
        inside.then(|| y as usize * SCREEN_W + x as usize)
    }

    pub fn clear(&mut self) {
        self.px.fill(false);
    }

    pub fn invert(&mut self) {
        self.px.iter_mut().for_each(|p| *p = !*p);
    }

    /// Draws a line with Bresenham's algorithm, so fast pointer strokes don't leave gaps.
    pub fn line(&mut self, (mut x0, mut y0): (i32, i32), (x1, y1): (i32, i32), on: bool) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.set(x0, y0, on);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Lights every pixel that is lit in `other`.
    pub fn merge(&mut self, other: &Bitmap) {
        for (a, b) in self.px.iter_mut().zip(&other.px) {
            *a |= *b;
        }
    }

    pub fn from_fn(mut f: impl FnMut(usize, usize) -> bool) -> Self {
        let mut bitmap = Self::default();
        for y in 0..SCREEN_H {
            for x in 0..SCREEN_W {
                bitmap.px[y * SCREEN_W + x] = f(x, y);
            }
        }
        bitmap
    }

    /// One string of 32 hex digits per row; the most significant bit is the leftmost pixel.
    pub fn to_rows(&self) -> Vec<String> {
        self.px
            .chunks(SCREEN_W)
            .map(|row| {
                row.chunks(8)
                    .map(|bits| {
                        let byte = bits.iter().fold(0u8, |acc, &b| (acc << 1) | b as u8);
                        format!("{byte:02x}")
                    })
                    .collect()
            })
            .collect()
    }

    pub fn from_rows(rows: &[String]) -> Option<Self> {
        if rows.len() != SCREEN_H {
            return None;
        }
        let mut bitmap = Self::default();
        for (y, row) in rows.iter().enumerate() {
            if row.len() != SCREEN_W / 4 || !row.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            for (i, pair) in row.as_bytes().chunks(2).enumerate() {
                let byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
                for bit in 0..8 {
                    bitmap.px[y * SCREEN_W + i * 8 + bit] = byte & (0x80 >> bit) != 0;
                }
            }
        }
        Some(bitmap)
    }
}

impl Serialize for Bitmap {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_rows().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Bitmap {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let rows = Vec::<String>::deserialize(d)?;
        Bitmap::from_rows(&rows).ok_or_else(|| serde::de::Error::custom("malformed screen image"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c = Rgb::new(0x12, 0xab, 0xff);
        assert_eq!(Rgb::parse(&c.to_hex()), Some(c));
        assert_eq!(Rgb::parse("fa0"), Some(Rgb::new(0xff, 0xaa, 0x00)));
        assert_eq!(Rgb::parse("+12345"), None);
        assert_eq!(Rgb::parse("#12345"), None);
    }

    #[test]
    fn hsv_primaries() {
        assert_eq!(Rgb::from_hsv(0.0, 1.0, 1.0), Rgb::new(255, 0, 0));
        assert_eq!(Rgb::from_hsv(120.0, 1.0, 1.0), Rgb::new(0, 255, 0));
        assert_eq!(Rgb::from_hsv(240.0, 1.0, 1.0), Rgb::new(0, 0, 255));
    }

    #[test]
    fn bitmap_rows_round_trip() {
        let bitmap = Bitmap::from_fn(|x, y| (x * 7 + y * 3) % 5 == 0);
        let rows = bitmap.to_rows();
        assert_eq!(rows.len(), SCREEN_H);
        assert!(rows.iter().all(|r| r.len() == 32));
        assert!(Bitmap::from_rows(&rows) == Some(bitmap));
    }

    #[test]
    fn leftmost_pixel_is_msb() {
        let mut bitmap = Bitmap::default();
        bitmap.set(0, 0, true);
        assert!(bitmap.to_rows()[0].starts_with("80"));
    }

    #[test]
    fn line_is_continuous() {
        let mut bitmap = Bitmap::default();
        bitmap.line((0, 0), (10, 3), true);
        for x in 0..=10 {
            assert!((0..4).any(|y| bitmap.get(x, y)), "gap at x={x}");
        }
        assert!(!bitmap.get(-1, 0));
    }
}
