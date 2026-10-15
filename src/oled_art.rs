//! Turning images and text into 128×40 one-bit bitmaps for the OLED.

use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, pango};

use crate::model::{Bitmap, SCREEN_H, SCREEN_W};

/// An image reduced to luminance (0..=255), composited over black.
pub struct Gray {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

pub fn load(file: &gio::File) -> Result<Gray, glib::Error> {
    let texture = gdk::Texture::from_file(file)?;
    let mut downloader = gdk::TextureDownloader::new(&texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let (width, height) = (texture.width() as usize, texture.height() as usize);
    let mut data = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = &bytes[y * stride..];
        for px in row[..width * 4].as_chunks::<4>().0 {
            let (r, g, b, a) = (px[0] as f32, px[1] as f32, px[2] as f32, px[3] as f32 / 255.0);
            data.push(((0.299 * r + 0.587 * g + 0.114 * b) * a).round() as u8);
        }
    }
    Ok(Gray { width, height, data })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// Whole image visible, black bars where the aspect ratio differs.
    Contain,
    /// Screen filled, edges cropped.
    Cover,
    /// Distorted to exactly 128×40.
    Stretch,
}

#[derive(Clone, Copy, Debug)]
pub struct Conversion {
    pub fit: Fit,
    pub dither: bool,
    /// Added to every pixel before thresholding, -255..=255.
    pub brightness: i32,
    pub invert: bool,
}

pub fn convert(src: &Gray, opts: Conversion) -> Bitmap {
    let (sw, sh) = (src.width as f64, src.height as f64);
    let (tw, th) = (SCREEN_W as f64, SCREEN_H as f64);
    let (scale_x, scale_y) = match opts.fit {
        Fit::Contain => {
            let s = (tw / sw).min(th / sh);
            (s, s)
        }
        Fit::Cover => {
            let s = (tw / sw).max(th / sh);
            (s, s)
        }
        Fit::Stretch => (tw / sw, th / sh),
    };
    let ox = (tw - sw * scale_x) / 2.0;
    let oy = (th - sh * scale_y) / 2.0;

    // Area-average every source pixel that falls inside each screen pixel.
    let mut gray = vec![0f32; SCREEN_W * SCREEN_H];
    for dy in 0..SCREEN_H {
        let sy0 = ((dy as f64 - oy) / scale_y).max(0.0);
        let sy1 = ((dy as f64 + 1.0 - oy) / scale_y).min(sh);
        if sy1 <= sy0 {
            continue;
        }
        let (iy0, iy1) = span(sy0, sy1, src.height);
        for dx in 0..SCREEN_W {
            let sx0 = ((dx as f64 - ox) / scale_x).max(0.0);
            let sx1 = ((dx as f64 + 1.0 - ox) / scale_x).min(sw);
            if sx1 <= sx0 {
                continue;
            }
            let (ix0, ix1) = span(sx0, sx1, src.width);
            let mut sum = 0u64;
            for y in iy0..iy1 {
                sum += src.data[y * src.width + ix0..y * src.width + ix1].iter().map(|&v| v as u64).sum::<u64>();
            }
            gray[dy * SCREEN_W + dx] = sum as f32 / ((iy1 - iy0) * (ix1 - ix0)) as f32;
        }
    }

    for v in &mut gray {
        *v = (*v + opts.brightness as f32).clamp(0.0, 255.0);
        if opts.invert {
            *v = 255.0 - *v;
        }
    }

    if opts.dither {
        // Floyd–Steinberg error diffusion
        let mut out = Bitmap::default();
        for y in 0..SCREEN_H {
            for x in 0..SCREEN_W {
                let i = y * SCREEN_W + x;
                let old = gray[i];
                let on = old >= 128.0;
                out.set(x as i32, y as i32, on);
                let err = old - if on { 255.0 } else { 0.0 };
                let mut spread = |dx: isize, dy: usize, w: f32| {
                    let nx = x as isize + dx;
                    if (0..SCREEN_W as isize).contains(&nx) && y + dy < SCREEN_H {
                        gray[(y + dy) * SCREEN_W + nx as usize] += err * w;
                    }
                };
                spread(1, 0, 7.0 / 16.0);
                spread(-1, 1, 3.0 / 16.0);
                spread(0, 1, 5.0 / 16.0);
                spread(1, 1, 1.0 / 16.0);
            }
        }
        out
    } else {
        Bitmap::from_fn(|x, y| gray[y * SCREEN_W + x] >= 128.0)
    }
}

/// Integer source range covering [a, b), at least one pixel wide.
fn span(a: f64, b: f64, len: usize) -> (usize, usize) {
    let start = (a.floor() as usize).min(len - 1);
    let end = (b.ceil() as usize).clamp(start + 1, len);
    (start, end)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct TextSpec<'a> {
    pub text: &'a str,
    pub font: &'a pango::FontDescription,
    pub align: Align,
    pub offset: (i32, i32),
}

/// Renders text without antialiasing, so every pixel is clearly on or off.
pub fn render_text(spec: &TextSpec) -> Bitmap {
    if spec.text.trim().is_empty() {
        return Bitmap::default();
    }
    let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::A8, SCREEN_W as i32, SCREEN_H as i32) else {
        return Bitmap::default();
    };
    {
        let Ok(cr) = cairo::Context::new(&surface) else {
            return Bitmap::default();
        };
        let context = pangocairo::functions::create_context(&cr);
        if let Ok(mut options) = cairo::FontOptions::new() {
            options.set_antialias(cairo::Antialias::None);
            options.set_hint_style(cairo::HintStyle::Full);
            pangocairo::functions::context_set_font_options(&context, Some(&options));
        }
        let layout = pango::Layout::new(&context);
        layout.set_font_description(Some(spec.font));
        layout.set_alignment(match spec.align {
            Align::Left => pango::Alignment::Left,
            Align::Center => pango::Alignment::Center,
            Align::Right => pango::Alignment::Right,
        });
        layout.set_text(spec.text);

        // Position by the ink rectangle so the text is optically centred.
        let (ink, _) = layout.pixel_extents();
        let x = match spec.align {
            Align::Left => -ink.x(),
            Align::Center => (SCREEN_W as i32 - ink.width()) / 2 - ink.x(),
            Align::Right => SCREEN_W as i32 - ink.width() - ink.x(),
        } + spec.offset.0;
        let y = (SCREEN_H as i32 - ink.height()) / 2 - ink.y() + spec.offset.1;
        cr.move_to(x as f64, y as f64);
        cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
        pangocairo::functions::show_layout(&cr, &layout);
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let Ok(data) = surface.data() else {
        return Bitmap::default();
    };
    Bitmap::from_fn(|x, y| data[y * stride + x] > 127)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(fit: Fit, dither: bool) -> Conversion {
        Conversion { fit, dither, brightness: 0, invert: false }
    }

    #[test]
    fn white_square_is_letterboxed() {
        let src = Gray { width: 10, height: 10, data: vec![255; 100] };
        let bitmap = convert(&src, opts(Fit::Contain, false));
        assert!(bitmap.get(64, 20));
        assert!(!bitmap.get(0, 20), "left bar should stay black");
        assert!(!bitmap.get(127, 20), "right bar should stay black");
    }

    #[test]
    fn cover_and_stretch_fill_the_screen() {
        let src = Gray { width: 10, height: 10, data: vec![255; 100] };
        for fit in [Fit::Cover, Fit::Stretch] {
            let bitmap = convert(&src, opts(fit, false));
            assert!(bitmap.get(0, 0) && bitmap.get(127, 39), "{fit:?}");
        }
    }

    #[test]
    fn dithered_mid_gray_is_about_half_lit() {
        let src = Gray { width: 128, height: 40, data: vec![128; 128 * 40] };
        let bitmap = convert(&src, opts(Fit::Stretch, true));
        let lit = (0..40).flat_map(|y| (0..128).map(move |x| (x, y))).filter(|&(x, y)| bitmap.get(x, y)).count();
        let ratio = lit as f64 / 5120.0;
        assert!((0.45..0.55).contains(&ratio), "ratio {ratio}");
    }

    #[test]
    fn text_is_centered_and_crisp() {
        let font = pango::FontDescription::from_string("Sans Bold 16");
        let bitmap = render_text(&TextSpec { text: "I", font: &font, align: Align::Center, offset: (0, 0) });
        let lit: Vec<i32> = (0..128).filter(|&x| (0..40).any(|y| bitmap.get(x, y))).collect();
        assert!(!lit.is_empty());
        let middle = (lit[0] + lit[lit.len() - 1]) / 2;
        assert!((60..=67).contains(&middle), "text centered at x={middle}");
        let blank = render_text(&TextSpec { text: "  ", font: &font, align: Align::Center, offset: (0, 0) });
        assert!(blank == Bitmap::default());
    }

    #[test]
    fn large_image_downscales() {
        let (w, h) = (1000, 500);
        let data = (0..w * h).map(|i| if (i % w) < w / 2 { 255 } else { 0 }).collect();
        let bitmap = convert(&Gray { width: w, height: h, data }, opts(Fit::Stretch, false));
        assert!(bitmap.get(10, 20) && !bitmap.get(120, 20));
    }
}
