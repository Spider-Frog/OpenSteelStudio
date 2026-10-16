// SPDX-License-Identifier: GPL-3.0-or-later

//! Animated lighting: a background effect plus an optional effect on key presses.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::layout::{self, Key};
use crate::model::{Lighting, Rgb};

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
pub fn render(fx: &LightingFx, keys: &[Key], colors: &Lighting, t: f64, presses: &[Press]) -> Lighting {
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
    out
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
        let out = render(&LightingFx::default(), &keys, &colors, 12.3, &[]);
        assert!(keys.iter().all(|k| out.get(k.led) == colors.get(k.led)));
    }

    #[test]
    fn breathing_dims_and_brightens() {
        let (keys, colors) = setup();
        let fx = LightingFx { background: Background::Breathing, ..Default::default() };
        let dark = render(&fx, &keys, &colors, 0.0, &[]).get(0x04);
        let bright = render(&fx, &keys, &colors, 2.0, &[]).get(0x04);
        assert!(dark.b < 20 && bright.b > 190, "{dark:?} {bright:?}");
    }

    #[test]
    fn wave_moves() {
        let (keys, colors) = setup();
        let fx = LightingFx { background: Background::Wave, ..Default::default() };
        assert_ne!(render(&fx, &keys, &colors, 0.0, &[]).get(0x04), render(&fx, &keys, &colors, 0.5, &[]).get(0x04));
    }

    #[test]
    fn pressed_key_lights_up_then_fades() {
        let (keys, colors) = setup();
        let fx = LightingFx { typing: Typing::Light, typing_color: Rgb::new(255, 0, 0), ..Default::default() };
        let press = [Press { led: 0x04, t: 1.0 }];
        let now = render(&fx, &keys, &colors, 1.0, &press);
        assert_eq!(now.get(0x04), Rgb::new(255, 0, 0));
        assert_eq!(now.get(0x05), colors.get(0x05), "other keys untouched");
        assert_eq!(render(&fx, &keys, &colors, 1.0 + fx.fade + 0.01, &press).get(0x04), colors.get(0x04));
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
        assert!(render(&fx, &keys, &colors, 0.0, &press).get(far).r < 10);
        assert!(render(&fx, &keys, &colors, distance / RIPPLE_SPEED, &press).get(far).r > 100);
    }

    #[test]
    fn starlight_twinkles_some_keys() {
        let (keys, colors) = setup();
        let fx = LightingFx { background: Background::Starlight, ..Default::default() };
        let out = render(&fx, &keys, &colors, 7.0, &[]);
        let lit = keys.iter().filter(|k| out.get(k.led).b > 20).count();
        assert!(lit > 3 && lit < keys.len() / 2, "{lit} keys lit");
    }
}
