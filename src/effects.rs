// SPDX-License-Identifier: GPL-3.0-or-later

//! Animated lighting: a background effect plus an optional effect on key presses.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::layout::{self, Key};
use crate::model::{Lighting, Rgb};
use crate::volume::{self, Level};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Background {
    /// Your key colors as painted.
    #[default]
    Static,
    /// Your key colors fading in and out.
    Breathing,
    /// A rainbow moving across the keyboard.
    Wave,
    /// All keys cycling through the rainbow together.
    Cycle,
    /// Your key colors twinkling on and off at random.
    Starlight,
}

impl Background {
    pub const ALL: [Background; 5] = [Self::Static, Self::Breathing, Self::Wave, Self::Cycle, Self::Starlight];
    pub const NAMES: [&str; 5] = ["Static", "Breathing", "Rainbow wave", "Color cycle", "Starlight"];

    pub fn uses_key_colors(self) -> bool {
        matches!(self, Self::Static | Self::Breathing | Self::Starlight)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    #[default]
    Right,
    Left,
    Down,
    Up,
    Out,
}

impl Direction {
    pub const ALL: [Direction; 5] = [Self::Right, Self::Left, Self::Down, Self::Up, Self::Out];
    pub const NAMES: [&str; 5] = ["Left to right", "Right to left", "Top to bottom", "Bottom to top", "From the center"];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Typing {
    #[default]
    Off,
    /// The pressed key flashes and fades out.
    Light,
    /// A ring spreads out from the pressed key.
    Ripple,
}

impl Typing {
    pub const ALL: [Typing; 3] = [Self::Off, Self::Light, Self::Ripple];
    pub const NAMES: [&str; 3] = ["Nothing", "Light up keys", "Ripple"];
}

/// Where the keys show the volume for a moment after it changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeBar {
    #[default]
    Off,
    FunctionKeys,
    NumberRow,
}

impl VolumeBar {
    pub const ALL: [VolumeBar; 3] = [Self::Off, Self::FunctionKeys, Self::NumberRow];
    pub const NAMES: [&str; 3] = ["Nothing", "Bar on F1–F12", "Bar on the number row"];

    /// The bar's keys, from empty to full.
    pub fn leds(self) -> &'static [u8] {
        match self {
            Self::Off => &[],
            Self::FunctionKeys => &[0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45],
            Self::NumberRow => &[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27],
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LightingFx {
    pub background: Background,
    pub direction: Direction,
    /// Animation speed multiplier, 1.0 = normal.
    pub speed: f64,
    pub typing: Typing,
    pub typing_color: Rgb,
    pub typing_rainbow: bool,
    /// Seconds a key press stays visible.
    pub fade: f64,
    pub volume_bar: VolumeBar,
}

impl Default for LightingFx {
    fn default() -> Self {
        Self {
            background: Background::Static,
            direction: Direction::Right,
            speed: 1.0,
            typing: Typing::Off,
            typing_color: Rgb::new(0xff, 0xff, 0xff),
            typing_rainbow: false,
            fade: 0.8,
            volume_bar: VolumeBar::Off,
        }
    }
}

impl LightingFx {
    /// Whether frames change over time, so they must be re-rendered continuously.
    pub fn is_animated(&self, now: f64, last_press: Option<f64>) -> bool {
        self.background != Background::Static
            || (self.typing != Typing::Off && last_press.is_some_and(|t| now - t <= self.fade + RIPPLE_WIDTH / RIPPLE_SPEED))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub led: u8,
    /// Seconds since the editor started.
    pub t: f64,
}

/// Ripple speed in key units per second, and ring thickness in key units.
const RIPPLE_SPEED: f64 = 14.0;
const RIPPLE_WIDTH: f64 = 1.2;

fn center(key: &Key) -> (f64, f64) {
    let (x0, y0, x1, y1) = key.bounds();
    ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
}

/// Cheap deterministic per-key randomness in 0..1.
fn hash01(a: u32, b: u32) -> f64 {
    let mut x = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f64 / 65535.0
}

/// Computes what every LED shows at time `t`.
pub fn render(fx: &LightingFx, keys: &[Key], colors: &Lighting, t: f64, presses: &[Press], volume: Option<Level>) -> Lighting {
    let speed = fx.speed.max(0.01);
    let mut centers = [None; 256];
    for key in keys {
        centers[key.led as usize] = Some(center(key));
    }
    let recent: Vec<&Press> = presses.iter().filter(|p| fx.typing != Typing::Off && t - p.t <= fx.fade + 1.0).collect();

    let mut out = Lighting::default();
    for key in keys {
        let led = key.led;
        let (cx, cy) = center(key);
        let own = colors.get(led);
        let base = match fx.background {
            Background::Static => own,
            Background::Breathing => {
                let b = 0.5 - 0.5 * (TAU * t * speed / 4.0).cos();
                Rgb::BLACK.mix(own, 0.04 + 0.96 * b)
            }
            Background::Wave => {
                let p = match fx.direction {
                    Direction::Right => cx,
                    Direction::Left => layout::WIDTH - cx,
                    Direction::Down => cy * 2.5,
                    Direction::Up => (layout::HEIGHT - cy) * 2.5,
                    Direction::Out => -((cx - layout::WIDTH / 2.0).powi(2) + ((cy - layout::HEIGHT / 2.0) * 1.5).powi(2)).sqrt(),
                };
                Rgb::from_hsv(p / layout::WIDTH * 360.0 - t * speed * 120.0, 1.0, 1.0)
            }
            Background::Cycle => Rgb::from_hsv(t * speed * 40.0, 1.0, 1.0),
            Background::Starlight => {
                // Every key twinkles once per cycle, at a random moment that changes every cycle.
                let period = 2.0 + 3.0 * hash01(led as u32, 1);
                let phase = t * speed / period + hash01(led as u32, 2);
                let cycle = phase.floor() as u32;
                let start = 0.7 * hash01(led as u32, cycle.wrapping_add(3));
                let local = (phase.fract() - start) / 0.3;
                let level = if (0.0..1.0).contains(&local) { (local * std::f64::consts::PI).sin() } else { 0.0 };
                Rgb::BLACK.mix(own, level)
            }
        };

        let mut level = 0.0f64;
        let mut tint = fx.typing_color;
        for press in &recent {
            let age = t - press.t;
            let fade = (1.0 - age / fx.fade.max(0.05)).clamp(0.0, 1.0);
            let hit = match fx.typing {
                Typing::Off => 0.0,
                Typing::Light => {
                    if press.led == led { fade } else { 0.0 }
                }
                Typing::Ripple => {
                    let Some((px, py)) = centers[press.led as usize] else { continue };
                    let d = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
                    let ring = (1.0 - (d - age * RIPPLE_SPEED).abs() / RIPPLE_WIDTH).max(0.0);
                    ring * fade
                }
            };
            if hit > level {
                level = hit;
                if fx.typing_rainbow {
                    tint = Rgb::from_hsv(press.t * 97.0, 1.0, 1.0);
                }
            }
        }
        out.set(led, base.mix(tint, level));
    }

    if let Some(volume) = volume.filter(|v| v.showing(t)) {
        volume_bar(fx.volume_bar, volume, t, &mut out);
    }
    out
}

/// Draws the volume over the bar's keys, fading out at the end of the hold time.
fn volume_bar(bar: VolumeBar, volume: Level, t: f64, out: &mut Lighting) {
    const EMPTY: Rgb = Rgb::new(14, 14, 14);
    const MUTED: Rgb = Rgb::new(90, 0, 0);
    let leds = bar.leds();
    let since = volume.changed_at.map_or(0.0, |at| t - at);
    let fade_out = ((volume::HOLD - since) / 0.4).clamp(0.0, 1.0);
    for (i, &led) in leds.iter().enumerate() {
        let position = i as f64 / (leds.len() - 1).max(1) as f64;
        let color = if volume.muted {
            MUTED
        } else {
            let fill = (volume.level.min(1.0) * leds.len() as f64 - i as f64).clamp(0.0, 1.0);
            EMPTY.mix(gradient(position), fill)
        };
        out.set(led, out.get(led).mix(color, fade_out));
    }
}

/// Green at the quiet end, through yellow, to red at the loud end.
fn gradient(position: f64) -> Rgb {
    const GREEN: Rgb = Rgb::new(0, 230, 70);
    const YELLOW: Rgb = Rgb::new(255, 200, 0);
    const RED: Rgb = Rgb::new(255, 30, 0);
    if position < 0.5 { GREEN.mix(YELLOW, position * 2.0) } else { YELLOW.mix(RED, position * 2.0 - 1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Region;

    fn setup() -> (Vec<Key>, Lighting) {
        let keys = layout::keys(Region::Ansi);
        let mut colors = Lighting::default();
        for k in &keys {
            colors.set(k.led, Rgb::new(0, 0, 200));
        }
        (keys, colors)
    }

    #[test]
    fn static_is_identity() {
        let (keys, colors) = setup();
        let out = render(&LightingFx::default(), &keys, &colors, 12.3, &[], None);
        assert!(keys.iter().all(|k| out.get(k.led) == colors.get(k.led)));
    }

    #[test]
    fn breathing_dims_and_brightens() {
        let (keys, colors) = setup();
        let fx = LightingFx { background: Background::Breathing, ..Default::default() };
        let dark = render(&fx, &keys, &colors, 0.0, &[], None).get(0x04);
        let bright = render(&fx, &keys, &colors, 2.0, &[], None).get(0x04);
        assert!(dark.b < 20 && bright.b > 190, "{dark:?} {bright:?}");
    }

    #[test]
    fn wave_moves() {
        let (keys, colors) = setup();
        let fx = LightingFx { background: Background::Wave, ..Default::default() };
        assert_ne!(render(&fx, &keys, &colors, 0.0, &[], None).get(0x04), render(&fx, &keys, &colors, 0.5, &[], None).get(0x04));
    }

    #[test]
    fn pressed_key_lights_up_then_fades() {
        let (keys, colors) = setup();
        let fx = LightingFx { typing: Typing::Light, typing_color: Rgb::new(255, 0, 0), ..Default::default() };
        let press = [Press { led: 0x04, t: 1.0 }];
        let now = render(&fx, &keys, &colors, 1.0, &press, None);
        assert_eq!(now.get(0x04), Rgb::new(255, 0, 0));
        assert_eq!(now.get(0x05), colors.get(0x05), "other keys untouched");
        assert_eq!(render(&fx, &keys, &colors, 1.0 + fx.fade + 0.01, &press, None).get(0x04), colors.get(0x04));
        assert!(fx.is_animated(1.5, Some(1.0)));
        assert!(!fx.is_animated(5.0, Some(1.0)));
    }

    #[test]
    fn ripple_reaches_neighbours_later() {
        let (keys, colors) = setup();
        let fx = LightingFx { typing: Typing::Ripple, typing_color: Rgb::new(255, 0, 0), fade: 2.0, ..Default::default() };
        let (from, far) = (0x0D, 0x2B); // J, Tab
        let at = |led: u8| center(keys.iter().find(|k| k.led == led).unwrap());
        let ((x0, y0), (x1, y1)) = (at(from), at(far));
        let distance = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let press = [Press { led: from, t: 0.0 }];
        assert!(render(&fx, &keys, &colors, 0.0, &press, None).get(far).r < 10);
        assert!(render(&fx, &keys, &colors, distance / RIPPLE_SPEED, &press, None).get(far).r > 100);
    }

    #[test]
    fn volume_bar_fills_and_fades() {
        let (keys, colors) = setup();
        let fx = LightingFx { volume_bar: VolumeBar::FunctionKeys, ..Default::default() };
        let half = Level { level: 0.5, muted: false, changed_at: Some(1.0) };
        let out = render(&fx, &keys, &colors, 1.5, &[], Some(half));
        let f = |n: u8| out.get(0x3A + n - 1); // F1..F12
        assert!(f(1).g > 200 && f(6).r > 200, "the lower half is lit: {:?} {:?}", f(1), f(6));
        assert_eq!(f(7), Rgb::new(14, 14, 14), "the upper half is empty");
        assert_eq!(out.get(0x04), colors.get(0x04), "other keys untouched");

        let muted = render(&fx, &keys, &colors, 1.5, &[], Some(Level { muted: true, ..half }));
        assert_eq!(muted.get(0x45), Rgb::new(90, 0, 0));
        let later = render(&fx, &keys, &colors, 1.0 + volume::HOLD + 0.1, &[], Some(half));
        assert_eq!(later.get(0x3A), colors.get(0x3A), "gone after the hold time");
        let off = render(&LightingFx::default(), &keys, &colors, 1.5, &[], Some(half));
        assert_eq!(off.get(0x3A), colors.get(0x3A), "no bar unless chosen");
    }

    #[test]
    fn starlight_twinkles_some_keys() {
        let (keys, colors) = setup();
        let fx = LightingFx { background: Background::Starlight, ..Default::default() };
        let out = render(&fx, &keys, &colors, 7.0, &[], None);
        let lit = keys.iter().filter(|k| out.get(k.led).b > 20).count();
        assert!(lit > 3 && lit < keys.len() / 2, "{lit} keys lit");
    }
}
