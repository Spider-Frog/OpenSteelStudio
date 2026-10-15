// SPDX-License-Identifier: GPL-3.0-or-later

//! Small Cairo helpers shared by the keyboard and screen views.

use gtk::cairo;

use crate::model::{Bitmap, Rgb, SCREEN_H, SCREEN_W};

pub fn set_rgb(cr: &cairo::Context, c: Rgb) {
    let (r, g, b) = c.to_f64();
    cr.set_source_rgb(r, g, b);
}

pub fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    polygon_path(cr, &[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], r);
}

/// Adds a closed path for an axis-aligned polygon with rounded corners.
pub fn polygon_path(cr: &cairo::Context, pts: &[(f64, f64)], radius: f64) {
    const KAPPA: f64 = 0.552; // cubic Bézier approximation of a quarter circle
    let n = pts.len();
    cr.new_path();
    for i in 0..n {
        let p = pts[i];
        let prev = pts[(i + n - 1) % n];
        let next = pts[(i + 1) % n];
        let r = radius.min(dist(p, prev) / 2.0).min(dist(p, next) / 2.0);
        let p1 = towards(p, prev, r);
        let p2 = towards(p, next, r);
        if i == 0 {
            cr.move_to(p1.0, p1.1);
        } else {
            cr.line_to(p1.0, p1.1);
        }
        cr.curve_to(
            p1.0 + (p.0 - p1.0) * KAPPA,
            p1.1 + (p.1 - p1.1) * KAPPA,
            p2.0 + (p.0 - p2.0) * KAPPA,
            p2.1 + (p.1 - p2.1) * KAPPA,
            p2.0,
            p2.1,
        );
    }
    cr.close_path();
}

/// Shrinks an axis-aligned, clockwise (in screen coordinates) polygon by `d` on every side.
pub fn inset(pts: &[(f64, f64)], d: f64) -> Vec<(f64, f64)> {
    let n = pts.len();
    let inward = |a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(f64::EPSILON);
        (-dy / len, dx / len)
    };
    (0..n)
        .map(|i| {
            let p = pts[i];
            let a = inward(pts[(i + n - 1) % n], p);
            let b = inward(p, pts[(i + 1) % n]);
            (p.0 + (a.0 + b.0) * d, p.1 + (a.1 + b.1) * d)
        })
        .collect()
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn towards(from: (f64, f64), to: (f64, f64), r: f64) -> (f64, f64) {
    let d = dist(from, to).max(f64::EPSILON);
    (from.0 + (to.0 - from.0) / d * r, from.1 + (to.1 - from.1) / d * r)
}

/// Paints `bitmap` into the given rectangle with crisp (nearest neighbour) scaling.
/// Colors are straight (non-premultiplied) RGBA in 0..=1.
pub fn bitmap(
    cr: &cairo::Context,
    bitmap: &Bitmap,
    (x, y, w, h): (f64, f64, f64, f64),
    on: (f64, f64, f64, f64),
    off: (f64, f64, f64, f64),
) {
    let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, SCREEN_W as i32, SCREEN_H as i32) else {
        return;
    };
    let stride = surface.stride() as usize;
    let premultiply = |(r, g, b, a): (f64, f64, f64, f64)| {
        // Cairo ARGB32 is a native-endian u32, i.e. B G R A in memory on little-endian machines.
        let px = u32::from_be_bytes([
            (a * 255.0) as u8,
            (r * a * 255.0) as u8,
            (g * a * 255.0) as u8,
            (b * a * 255.0) as u8,
        ]);
        px.to_ne_bytes()
    };
    let (on, off) = (premultiply(on), premultiply(off));
    if let Ok(mut data) = surface.data() {
        for py in 0..SCREEN_H {
            for px in 0..SCREEN_W {
                let o = py * stride + px * 4;
                let value = if bitmap.get(px as i32, py as i32) { on } else { off };
                data[o..o + 4].copy_from_slice(&value);
            }
        }
    }
    let pattern = cairo::SurfacePattern::create(&surface);
    pattern.set_filter(cairo::Filter::Nearest);
    let _ = cr.save();
    cr.translate(x, y);
    cr.scale(w / SCREEN_W as f64, h / SCREEN_H as f64);
    let _ = cr.set_source(&pattern);
    cr.rectangle(0.0, 0.0, SCREEN_W as f64, SCREEN_H as f64);
    let _ = cr.fill();
    let _ = cr.restore();
}
