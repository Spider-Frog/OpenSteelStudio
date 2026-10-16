// SPDX-License-Identifier: GPL-3.0-or-later

//! Animated screen content: clock, scrolling text, GIF playback and typing displays.

use gtk::{glib, pango};
use serde::{Deserialize, Serialize};

use crate::effects::Press;
use crate::model::{Bitmap, SCREEN_H, SCREEN_W};
use crate::oled_art::{self, Align, Strip, TextSpec};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenMode {
    /// The image drawn on the Screen page.
    #[default]
    Image,
    Clock,
    Marquee,
    Animation,
}

impl ScreenMode {
    pub const ALL: [ScreenMode; 4] = [Self::Image, Self::Clock, Self::Marquee, Self::Animation];
    pub const NAMES: [&str; 4] = ["Image", "Clock", "Scrolling text", "Animation (GIF)"];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockStyle {
    #[default]
    Time,
    Seconds,
    Date,
}

impl ClockStyle {
    pub const ALL: [ClockStyle; 3] = [Self::Time, Self::Seconds, Self::Date];
    pub const NAMES: [&str; 3] = ["Time", "Time with seconds", "Time and date"];
}

/// What the screen shows for a few seconds after a key press. Deliberately never shows which
/// keys were pressed, so passwords can't be read off the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenTyping {
    #[default]
    Off,
    /// Words per minute plus a bar graph of the last seconds.
    Speed,
    /// Keys pressed since the app started.
    Counter,
}

impl ScreenTyping {
    pub const ALL: [ScreenTyping; 3] = [Self::Off, Self::Speed, Self::Counter];
    pub const NAMES: [&str; 3] = ["Nothing", "Typing speed", "Key counter"];
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenFx {
    pub mode: ScreenMode,
    pub clock_style: ClockStyle,
    pub clock_24h: bool,
    pub clock_font: String,
    pub marquee_text: String,
    pub marquee_font: String,
    pub marquee_speed: f64,
    pub animation_speed: f64,
    pub typing: ScreenTyping,
    /// Seconds the typing display stays up after the last key press.
    pub typing_hold: f64,
}

impl Default for ScreenFx {
    fn default() -> Self {
        Self {
            mode: ScreenMode::Image,
            clock_style: ClockStyle::Time,
            clock_24h: true,
            clock_font: "Sans Bold 22".into(),
            marquee_text: "Hello from OpenSteelStudio".into(),
            marquee_font: "Sans Bold 16".into(),
            marquee_speed: 1.0,
            animation_speed: 1.0,
            typing: ScreenTyping::Off,
            typing_hold: 3.0,
        }
    }
}

impl ScreenFx {
    pub fn typing_visible(&self, now: f64, last_press: Option<f64>) -> bool {
        self.typing != ScreenTyping::Off && last_press.is_some_and(|t| now - t <= self.typing_hold)
    }

    pub fn is_animated(&self, now: f64, last_press: Option<f64>, animation: &Animation) -> bool {
        self.typing_visible(now, last_press)
            || match self.mode {
                ScreenMode::Image => false,
                ScreenMode::Clock | ScreenMode::Marquee => true,
                ScreenMode::Animation => !animation.frames.is_empty(),
            }
    }
}

/// A converted GIF: one bitmap per frame and how long each frame shows.
#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
pub struct Animation {
    pub frames: Vec<Bitmap>,
    pub delays_ms: Vec<u32>,
}

impl Animation {
    pub fn duration_ms(&self) -> u64 {
        self.delays_ms.iter().map(|&d| d as u64).sum()
    }

    pub fn frame_at(&self, ms: f64) -> Option<&Bitmap> {
        let total = self.duration_ms();
        if total == 0 || self.frames.len() != self.delays_ms.len() {
            return self.frames.first();
        }
        let mut at = (ms.max(0.0) as u64) % total;
        for (frame, &delay) in self.frames.iter().zip(&self.delays_ms) {
            if at < delay as u64 {
                return Some(frame);
            }
            at -= delay as u64;
        }
        self.frames.last()
    }
}

/// Everything a frame depends on besides the settings.
pub struct Inputs<'a> {
    pub image: &'a Bitmap,
    pub animation: &'a Animation,
    /// Seconds since the editor started.
    pub t: f64,
    pub presses: &'a [Press],
    pub total_presses: u64,
}

/// Renders reused between frames.
#[derive(Default)]
pub struct Cache {
    strip: Option<(String, String, Strip)>,
    clock: Option<(String, Bitmap)>,
}

pub fn render(fx: &ScreenFx, input: &Inputs, cache: &mut Cache) -> Bitmap {
    if fx.typing_visible(input.t, input.presses.last().map(|p| p.t)) {
        return match fx.typing {
            ScreenTyping::Speed => typing_speed(input.presses, input.t),
            ScreenTyping::Counter => key_counter(input.total_presses),
            ScreenTyping::Off => unreachable!(),
        };
    }
    match fx.mode {
        ScreenMode::Image => input.image.clone(),
        ScreenMode::Clock => match glib::DateTime::now_local() {
            Ok(now) => clock(fx, &now, cache),
            Err(_) => Bitmap::default(),
        },
        ScreenMode::Marquee => marquee(fx, input.t, cache),
        ScreenMode::Animation => {
            input.animation.frame_at(input.t * 1000.0 * fx.animation_speed).cloned().unwrap_or_default()
        }
    }
}

pub fn clock_text(fx: &ScreenFx, now: &glib::DateTime) -> (String, Option<String>) {
    let pattern = match (fx.clock_style, fx.clock_24h) {
        (ClockStyle::Seconds, true) => "%H:%M:%S",
        (ClockStyle::Seconds, false) => "%l:%M:%S %p",
        (_, true) => "%H:%M",
        (_, false) => "%l:%M %p",
    };
    let format = |p: &str| now.format(p).map(|s| s.trim().replace("  ", " ")).unwrap_or_default();
    let date = (fx.clock_style == ClockStyle::Date).then(|| format("%a %e %b"));
    (format(pattern), date)
}

fn clock(fx: &ScreenFx, now: &glib::DateTime, cache: &mut Cache) -> Bitmap {
    let (time, date) = clock_text(fx, now);
    let key = format!("{time}|{date:?}|{}", fx.clock_font);
    if let Some((cached, bitmap)) = &cache.clock
        && *cached == key
    {
        return bitmap.clone();
    }
    let font = pango::FontDescription::from_string(&fx.clock_font);
    let bitmap = match &date {
        None => oled_art::render_text_fitted(&time, &font, 124, (0, 0)),
        Some(date) => {
            let mut bitmap = oled_art::render_text_fitted(&time, &font, 124, (0, -6));
            let small = pango::FontDescription::from_string("Sans 8");
            bitmap.merge(&oled_art::render_text(&TextSpec { text: date, font: &small, align: Align::Center, offset: (0, 14) }));
            bitmap
        }
    };
    cache.clock = Some((key, bitmap.clone()));
    bitmap
}

/// Pixels per second at speed 1.0.
const MARQUEE_PX_PER_S: f64 = 30.0;

fn marquee(fx: &ScreenFx, t: f64, cache: &mut Cache) -> Bitmap {
    if fx.marquee_text.trim().is_empty() {
        return Bitmap::default();
    }
    let stale = cache.strip.as_ref().is_none_or(|(text, font, _)| *text != fx.marquee_text || *font != fx.marquee_font);
    if stale {
        let strip = oled_art::render_strip(&fx.marquee_text, &pango::FontDescription::from_string(&fx.marquee_font));
        cache.strip = Some((fx.marquee_text.clone(), fx.marquee_font.clone(), strip));
    }
    let Some((_, _, strip)) = &cache.strip else { return Bitmap::default() };
    // The text enters on the right, leaves on the left, then starts over.
    let period = (strip.width + SCREEN_W) as f64;
    let travelled = (t * MARQUEE_PX_PER_S * fx.marquee_speed).rem_euclid(period);
    let left = SCREEN_W as i32 - travelled as i32;
    Bitmap::from_fn(|x, y| strip.get(x as i32 - left, y as i32))
}

/// Words per minute (5 keys per word) over the last 10 seconds, or over the time since typing
/// started if that is shorter, but at least 3 seconds so the first few keys don't spike it.
pub fn words_per_minute(presses: &[Press], t: f64) -> u32 {
    let recent: Vec<&Press> = presses.iter().filter(|p| (0.0..10.0).contains(&(t - p.t))).collect();
    let Some(first) = recent.first() else { return 0 };
    let window = (t - first.t).clamp(3.0, 10.0);
    (recent.len() as f64 / window * 60.0 / 5.0).round() as u32
}

fn typing_speed(presses: &[Press], t: f64) -> Bitmap {
    let number = pango::FontDescription::from_string("Sans Bold 20");
    let label = pango::FontDescription::from_string("Sans Bold 7");
    let wpm = words_per_minute(presses, t).to_string();
    let mut bitmap = oled_art::render_text(&TextSpec { text: &wpm, font: &number, align: Align::Left, offset: (3, -5) });
    bitmap.merge(&oled_art::render_text(&TextSpec { text: "WPM", font: &label, align: Align::Left, offset: (4, 14) }));

    // Keys per second over the last 16 seconds, newest on the right.
    const BARS: usize = 16;
    let mut buckets = [0u32; BARS];
    for p in presses {
        let age = t - p.t;
        if (0.0..BARS as f64).contains(&age) {
            buckets[BARS - 1 - age as usize] += 1;
        }
    }
    let peak = buckets.iter().copied().max().unwrap_or(0).max(4);
    let (left, bottom) = (62, SCREEN_H as i32 - 2);
    for (i, &n) in buckets.iter().enumerate() {
        let height = (n * 32 / peak) as i32;
        let x0 = left + i as i32 * 4;
        for x in x0..x0 + 3 {
            for y in bottom - height..bottom {
                bitmap.set(x, y, true);
            }
        }
    }
    bitmap.line((left, bottom + 1), (left + BARS as i32 * 4 - 2, bottom + 1), true);
    bitmap
}

fn key_counter(total: u64) -> Bitmap {
    let number = pango::FontDescription::from_string("Sans Bold 20");
    let label = pango::FontDescription::from_string("Sans Bold 7");
    let mut bitmap = oled_art::render_text_fitted(&total.to_string(), &number, 124, (0, -5));
    let text = if total == 1 { "KEY PRESSED" } else { "KEYS PRESSED" };
    bitmap.merge(&oled_art::render_text(&TextSpec { text, font: &label, align: Align::Center, offset: (0, 14) }));
    bitmap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(bitmap: &Bitmap) -> usize {
        (0..SCREEN_H as i32).flat_map(|y| (0..SCREEN_W as i32).map(move |x| (x, y))).filter(|&(x, y)| bitmap.get(x, y)).count()
    }

    #[test]
    fn animation_frames_follow_delays() {
        let a = Bitmap::from_fn(|x, _| x == 0);
        let b = Bitmap::from_fn(|x, _| x == 1);
        let anim = Animation { frames: vec![a.clone(), b.clone()], delays_ms: vec![100, 300] };
        assert!(anim.frame_at(50.0) == Some(&a));
        assert!(anim.frame_at(150.0) == Some(&b));
        assert!(anim.frame_at(450.0) == Some(&a), "loops");
    }

    #[test]
    fn clock_formats() {
        let at = glib::DateTime::from_local(2026, 10, 16, 9, 5, 7.0).unwrap();
        let mut fx = ScreenFx::default();
        assert_eq!(clock_text(&fx, &at).0, "09:05");
        fx.clock_style = ClockStyle::Seconds;
        assert_eq!(clock_text(&fx, &at).0, "09:05:07");
        fx.clock_24h = false;
        assert!(clock_text(&fx, &at).0.starts_with("9:05:07"));
        fx.clock_style = ClockStyle::Date;
        assert!(clock_text(&fx, &at).1.is_some());
    }

    #[test]
    fn marquee_scrolls() {
        let fx = ScreenFx { mode: ScreenMode::Marquee, ..Default::default() };
        let mut cache = Cache::default();
        let a = marquee(&fx, 2.0, &mut cache);
        let b = marquee(&fx, 2.5, &mut cache);
        assert!(lit(&a) > 0 && a != b);
    }

    #[test]
    fn typing_display_shows_then_hides() {
        let fx = ScreenFx { typing: ScreenTyping::Speed, ..Default::default() };
        let presses: Vec<Press> = (0..20).map(|i| Press { led: 4, t: 10.0 + i as f64 * 0.2 }).collect();
        assert_eq!(words_per_minute(&presses, 14.0), 60); // 20 keys in 4 s
        assert_eq!(words_per_minute(&presses[..2], 10.3), 8, "short bursts use the 3 s minimum");
        assert_eq!(words_per_minute(&presses, 40.0), 0);
        let image = Bitmap::default();
        let animation = Animation::default();
        let input = |t| Inputs { image: &image, animation: &animation, t, presses: &presses, total_presses: 20 };
        let mut cache = Cache::default();
        assert!(lit(&render(&fx, &input(14.0), &mut cache)) > 0);
        assert!(lit(&render(&fx, &input(30.0), &mut cache)) == 0, "back to the blank image");
    }
}
