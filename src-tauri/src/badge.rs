//! Unread-count badge on the taskbar icon (Windows *overlay icon*, like
//! Telegram) and on the tray icon.
//!
//! The badge is rasterised in-process into an RGBA buffer — no image or font
//! crate is pulled in, keeping the binary small (product principle #3). The
//! base tray icon is already a `tauri::image::Image` with raw RGBA, so no PNG
//! decoding is needed either; we just composite our own pixels onto it.
//!
//! The count is the sum of unread across all `role='inbox'` folders
//! ([`crate::db::queries::total_inbox_unread`]); zero means no badge.

use tauri::image::Image;
use tauri::{AppHandle, Manager};

/// Notification red. Deliberately **not** the violet `--accent`, which is
/// reserved exclusively for AI features.
const BADGE_RGB: [u8; 3] = [0xE5, 0x48, 0x4D];

/// Recompute the inbox unread total and repaint the taskbar overlay + tray
/// icon. Cheap (one indexed `SUM` plus a tiny raster), so it is safe to call on
/// every mail change.
pub async fn refresh(app: &AppHandle) {
    let count = {
        let state = app.state::<crate::state::AppState>();
        state
            .db
            .call(|conn| crate::db::queries::total_inbox_unread(conn))
            .await
            .unwrap_or(0)
            .max(0)
    };

    // Taskbar overlay icon (main window). `None` removes it.
    #[cfg(target_os = "windows")]
    if let Some(window) = app.get_webview_window("main") {
        let overlay = (count > 0).then(|| render_overlay(count));
        let _ = window.set_overlay_icon(overlay);
    }

    // Tray icon: composite the badge onto the base icon, or restore the base
    // when there is nothing unread.
    if let (Some(tray), Some(base)) = (app.tray_by_id("main"), app.default_window_icon()) {
        let icon = if count > 0 {
            render_tray(base, count)
        } else {
            base.clone()
        };
        let _ = tray.set_icon(Some(icon));
    }
}

/// The text shown in the badge: the count, or `99+` once it overflows.
fn badge_text(count: i64) -> String {
    if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    }
}

/// A standalone ~32×32 overlay: a filled red disc with the count centred.
/// Windows scales it down into the corner of the taskbar button.
fn render_overlay(count: i64) -> Image<'static> {
    const SIZE: u32 = 32;
    let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];
    draw_badge(&mut buf, SIZE, SIZE, 0, 0, SIZE, &badge_text(count));
    Image::new_owned(buf, SIZE, SIZE)
}

/// The base icon with a smaller badge composited into the bottom-right corner.
fn render_tray(base: &Image, count: i64) -> Image<'static> {
    let (w, h) = (base.width(), base.height());
    let mut buf = base.rgba().to_vec();
    // Badge diameter ~60% of the icon — big enough to read a two-digit count.
    let diam = (w.min(h) * 60 / 100).max(1);
    let (x0, y0) = (w - diam, h - diam);
    draw_badge(&mut buf, w, h, x0, y0, diam, &badge_text(count));
    Image::new_owned(buf, w, h)
}

/// Draw a filled anti-aliased red disc of `diam` at `(x0, y0)` within an RGBA
/// buffer of size `img_w × img_h`, then centre `text` on it in white.
fn draw_badge(buf: &mut [u8], img_w: u32, img_h: u32, x0: u32, y0: u32, diam: u32, text: &str) {
    let r = diam as f32 / 2.0;
    let cx = x0 as f32 + r;
    let cy = y0 as f32 + r;
    for py in y0..(y0 + diam).min(img_h) {
        for px in x0..(x0 + diam).min(img_w) {
            let dx = px as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;
            // Coverage falls off over the outermost pixel for a smooth edge.
            let cov = (r - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(buf, img_w, px, py, BADGE_RGB, cov);
            }
        }
    }
    draw_text(buf, img_w, x0, y0, diam, text);
}

/// Centre `text` within the badge box, scaled to fit, painted white.
fn draw_text(buf: &mut [u8], img_w: u32, x0: u32, y0: u32, diam: u32, text: &str) {
    let glyphs: Vec<&[u8; GLYPH_H]> = text.bytes().filter_map(glyph).collect();
    let n = glyphs.len() as u32;
    if n == 0 {
        return;
    }
    // Fit the text inside the disc with a little padding on every side.
    let avail_w = (diam as f32 * 0.78) as u32;
    let avail_h = (diam as f32 * 0.60) as u32;
    let units_w = n * GLYPH_W as u32 + (n - 1); // 1-column gap between glyphs
    let scale = (avail_w / units_w).min(avail_h / GLYPH_H as u32).max(1);
    let text_w = units_w * scale;
    let text_h = GLYPH_H as u32 * scale;
    let mut gx = x0 + (diam - text_w) / 2;
    let gy = y0 + (diam - text_h) / 2;
    for glyph in glyphs {
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..GLYPH_W as u32 {
                if bits & (1 << (GLYPH_W as u32 - 1 - col)) != 0 {
                    fill_block(buf, img_w, gx + col * scale, gy + row as u32 * scale, scale);
                }
            }
        }
        gx += (GLYPH_W as u32 + 1) * scale;
    }
}

/// Paint a `scale × scale` opaque-white block at `(x, y)`.
fn fill_block(buf: &mut [u8], img_w: u32, x: u32, y: u32, scale: u32) {
    for sy in 0..scale {
        for sx in 0..scale {
            blend(buf, img_w, x + sx, y + sy, [0xFF, 0xFF, 0xFF], 1.0);
        }
    }
}

/// Source-over composite of an opaque `color` at coverage `a` onto one pixel.
fn blend(buf: &mut [u8], img_w: u32, x: u32, y: u32, color: [u8; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    let i = ((y * img_w + x) * 4) as usize;
    if i + 3 >= buf.len() {
        return;
    }
    for c in 0..3 {
        let src = color[c] as f32;
        let dst = buf[i + c] as f32;
        buf[i + c] = (src * a + dst * (1.0 - a)).round() as u8;
    }
    let da = buf[i + 3] as f32 / 255.0;
    buf[i + 3] = ((a + da * (1.0 - a)) * 255.0).round() as u8;
}

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;

/// Map an ASCII byte to its 5×7 glyph (digits and `+` only).
fn glyph(b: u8) -> Option<&'static [u8; GLYPH_H]> {
    match b {
        b'0'..=b'9' => Some(&FONT[(b - b'0') as usize]),
        b'+' => Some(&FONT[10]),
        _ => None,
    }
}

/// 5×7 bitmap font for `0`–`9` and `+`. Each row uses the low 5 bits, MSB = left.
#[rustfmt::skip]
const FONT: [[u8; GLYPH_H]; 11] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E], // 0
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E], // 1
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F], // 2
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E], // 3
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02], // 4
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E], // 5
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E], // 6
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08], // 7
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E], // 8
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C], // 9
    [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00], // +
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_caps_at_99_plus() {
        assert_eq!(badge_text(1), "1");
        assert_eq!(badge_text(42), "42");
        assert_eq!(badge_text(99), "99");
        assert_eq!(badge_text(100), "99+");
        assert_eq!(badge_text(9999), "99+");
    }

    #[test]
    fn overlay_has_expected_size_and_paints_pixels() {
        let img = render_overlay(5);
        assert_eq!(img.width(), 32);
        assert_eq!(img.height(), 32);
        let rgba = img.rgba();
        assert_eq!(rgba.len(), 32 * 32 * 4);
        // The disc paints opaque red pixels...
        assert!(rgba
            .chunks(4)
            .any(|p| p[3] > 200 && p[0] > 200 && p[1] < 120));
        // ...and the digit paints white pixels.
        assert!(rgba
            .chunks(4)
            .any(|p| p[0] > 240 && p[1] > 240 && p[2] > 240 && p[3] > 200));
    }

    #[test]
    fn tray_preserves_size_and_stamps_corner() {
        // A fully transparent 32×32 base.
        let base = Image::new_owned(vec![0u8; 32 * 32 * 4], 32, 32);
        let out = render_tray(&base, 3);
        assert_eq!((out.width(), out.height()), (32, 32));
        let rgba = out.rgba();
        // The bottom-right corner now carries the red badge.
        let corner = ((28 * 32 + 28) * 4) as usize;
        assert!(rgba[corner + 3] > 0, "corner pixel should be painted");
        // The top-left corner is untouched (still transparent).
        assert_eq!(rgba[0..4], [0, 0, 0, 0]);
    }
}
