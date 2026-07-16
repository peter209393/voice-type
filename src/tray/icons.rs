//! Procedurally-rendered tray icons.
//!
//! Icons are drawn into raw RGBA buffers at runtime so the tray appearance
//! does NOT depend on the system icon theme. Using themed icon names like
//! `microphone-sensitivity-muted` caused the rendered tray icon to change
//! whenever the active icon theme was swapped, updated, or partially missing
//! (common on minimal WMs such as sway/waybar).

use crate::UiState;

pub const ICON_SIZE: u32 = 32;

/// Raw RGBA8 icon: bytes are `[R, G, B, A]` per pixel, row-major, top-down.
#[derive(Clone)]
pub struct IconData {
    pub width: i32,
    pub height: i32,
    pub rgba: Vec<u8>,
}

impl IconData {
    /// Reorder to ksni's ARGB32, network byte order (big-endian `[A, R, G, B]`).
    #[cfg(not(feature = "gtk-tray"))]
    pub fn to_argb32_be(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rgba.len());
        for px in self.rgba.chunks_exact(4) {
            out.push(px[3]); // A
            out.push(px[0]); // R
            out.push(px[1]); // G
            out.push(px[2]); // B
        }
        out
    }
}

struct Canvas {
    size: u32,
    pixels: Vec<[u8; 4]>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            size: ICON_SIZE,
            pixels: vec![[0, 0, 0, 0]; (ICON_SIZE * ICON_SIZE) as usize],
        }
    }

    #[inline]
    fn within(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.size && (y as u32) < self.size
    }

    /// Overwrite pixel (used for carving transparent holes).
    fn set(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if self.within(x, y) {
            let i = (y as u32 * self.size + x as u32) as usize;
            self.pixels[i] = c;
        }
    }

    /// Source-over blend.
    fn blend(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if !self.within(x, y) {
            return;
        }
        let i = (y as u32 * self.size + x as u32) as usize;
        let dst = &mut self.pixels[i];
        let sa = c[3] as f32 / 255.0;
        if sa <= 0.0 {
            return;
        }
        let inv = 1.0 - sa;
        dst[0] = (c[0] as f32 * sa + dst[0] as f32 * inv).round() as u8;
        dst[1] = (c[1] as f32 * sa + dst[1] as f32 * inv).round() as u8;
        dst[2] = (c[2] as f32 * sa + dst[2] as f32 * inv).round() as u8;
        let da = dst[3] as f32 / 255.0;
        dst[3] = ((sa + da * inv) * 255.0).round() as u8;
    }

    /// Anti-aliased filled disk via source-over compositing.
    fn disk(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
        if r <= 0.0 {
            return;
        }
        let lo = (r.ceil() as i32) + 1;
        let s = self.size as i32;
        let x0 = ((cx - lo as f32).ceil() as i32).max(0);
        let y0 = ((cy - lo as f32).ceil() as i32).max(0);
        let x1 = ((cx + lo as f32).floor() as i32).min(s - 1);
        let y1 = ((cy + lo as f32).floor() as i32).min(s - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (r - d + 0.5).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let mut cc = c;
                cc[3] = ((c[3] as f32) * cov).round() as u8;
                self.blend(x, y, cc);
            }
        }
    }

    /// Hard-edged filled disk that overwrites existing pixels.
    fn carve(&mut self, cx: f32, cy: f32, r: f32) {
        if r <= 0.0 {
            return;
        }
        let s = self.size as i32;
        let r2 = r * r;
        let x0 = ((cx - r).ceil() as i32).max(0);
        let y0 = ((cy - r).ceil() as i32).max(0);
        let x1 = ((cx + r).floor() as i32).min(s - 1);
        let y1 = ((cy + r).floor() as i32).min(s - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                if dx * dx + dy * dy <= r2 {
                    self.set(x, y, [0, 0, 0, 0]);
                }
            }
        }
    }

    /// Thick line drawn as overlapping disks (round caps, AA edges).
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, c: [u8; 4]) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = dx.hypot(dy);
        let steps = (len.ceil() as i32).max(1);
        let rad = thickness / 2.0;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            self.disk(x1 + dx * t, y1 + dy * t, rad, c);
        }
    }

    /// Three horizontal dots centered at (cx, cy).
    fn dots(&mut self, cx: f32, cy: f32, c: [u8; 4]) {
        for off in [-1.0, 0.0, 1.0] {
            self.disk(cx + off * 5.0, cy, 1.8, c);
        }
    }

    /// Check mark inside the centered bounding square of half-size `s`.
    fn check(&mut self, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
        self.line(cx - s, cy, cx - s * 0.25, cy + s * 0.85, 2.8, c);
        self.line(cx - s * 0.25, cy + s * 0.85, cx + s, cy - s * 0.85, 2.8, c);
    }

    /// X mark.
    fn cross(&mut self, cx: f32, cy: f32, s: f32, c: [u8; 4]) {
        self.line(cx - s, cy - s, cx + s, cy + s, 2.8, c);
        self.line(cx + s, cy - s, cx - s, cy + s, 2.8, c);
    }

    fn finish(self) -> IconData {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for p in self.pixels {
            rgba.push(p[0]);
            rgba.push(p[1]);
            rgba.push(p[2]);
            rgba.push(p[3]);
        }
        IconData {
            width: self.size as i32,
            height: self.size as i32,
            rgba,
        }
    }
}

const GRAY: [u8; 4] = [149, 156, 167, 255];
const RED: [u8; 4] = [224, 64, 64, 255];
const AMBER: [u8; 4] = [240, 168, 32, 255];
const GREEN: [u8; 4] = [56, 184, 96, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Render the icon for the given UI state. Deterministic and theme-independent.
pub fn icon_for_state(state: &UiState) -> IconData {
    let mut cv = Canvas::new();
    let cx = ICON_SIZE as f32 / 2.0;
    let cy = ICON_SIZE as f32 / 2.0;

    match state {
        UiState::Idle => {
            // Gray ring: filled disk with a transparent center.
            cv.disk(cx, cy, 13.0, GRAY);
            cv.carve(cx, cy, 9.5);
        }
        UiState::Recording { .. } => {
            // Solid red dot (classic REC indicator).
            cv.disk(cx, cy, 10.5, RED);
        }
        UiState::Transcribing { .. } => {
            cv.disk(cx, cy, 13.0, AMBER);
            cv.dots(cx, cy, WHITE);
        }
        UiState::Done { .. } => {
            cv.disk(cx, cy, 13.0, GREEN);
            cv.check(cx, cy - 1.0, 5.5, WHITE);
        }
        UiState::Error { .. } => {
            cv.disk(cx, cy, 13.0, RED);
            cv.cross(cx, cy, 5.5, WHITE);
        }
    }

    cv.finish()
}
