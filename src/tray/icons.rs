//! Procedurally-rendered tray icons.
//!
//! Icons are drawn into raw RGBA buffers at runtime so the tray appearance
//! does NOT depend on the system icon theme. Using themed icon names like
//! `microphone-sensitivity-muted` caused the rendered tray icon to change
//! whenever the active icon theme was swapped, updated, or partially missing
//! (common on minimal WMs such as sway/waybar).
//!
//! Rendering uses signed distance fields (SDF) with supersampling: shapes are
//! described analytically in a 32-unit design space, then rasterized at 64px
//! with 3x3 subsamples per pixel. That yields smooth anti-aliased edges and
//! gradients that stay crisp when the tray host downscales the pixmap
//! (waybar keeps ~16-24px logical icons, often on HiDPI outputs).

use crate::UiState;

/// Native pixmap size shipped over DBus.
///
/// 64px is plenty for tray hosts: they pick the closest/largest pixmap and
/// scale down, which preserves quality far better than scaling up a 32px one.
pub const ICON_SIZE: u32 = 64;

/// All shape math happens in this coordinate space (1 unit = ICON_SIZE/32 px).
const DESIGN: f32 = 32.0;

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
        for i in (0..self.rgba.len()).step_by(4) {
            out.push(self.rgba[i + 3]); // A
            out.push(self.rgba[i]); // R
            out.push(self.rgba[i + 1]); // G
            out.push(self.rgba[i + 2]); // B
        }
        out
    }
}

// ---------------------------------------------------------------------------
// SDF primitives (design space, y grows downward)
// ---------------------------------------------------------------------------

/// Distance to a filled circle.
fn sd_circle(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((x - cx).hypot(y - cy)) - r
}

/// Distance to a filled rounded box (capsule when r == min(half-extents)).
fn sd_round_box(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - hx + r;
    let qy = (y - cy).abs() - hy + r;
    qx.max(qy).min(0.0).hypot(qx.max(qy).max(0.0)) - r
}

/// Distance to a line segment.
fn sd_segment(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let abx = bx - ax;
    let aby = by - ay;
    let apx = x - ax;
    let apy = y - ay;
    let t = (apx * abx + apy * aby) / (abx * abx + aby * aby).max(1e-9);
    let t = t.clamp(0.0, 1.0);
    (apx - abx * t).hypot(apy - aby * t)
}

/// Turn a filled distance into a stroked outline of thickness `2 * half`.
fn stroke(d: f32, half: f32) -> f32 {
    d.abs() - half
}

/// Distance to an arc stroke (centered `cx,cy`, radius `r`, sweep `a0..a1`
/// radians in screen coordinates, i.e. 90° points to the bottom).
///
/// Implemented as a dense polyline of segments; the union approximates a
/// stroked arc with slightly-rounded joins, which looks good at icon scale.
#[allow(clippy::too_many_arguments)] // SDF helper: raw geometric parameters read clearer flat
fn sd_arc(x: f32, y: f32, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, half: f32) -> f32 {
    let n = 40;
    let mut d = f32::INFINITY;
    for i in 0..n {
        let t0 = a0 + (a1 - a0) * (i as f32 / n as f32);
        let t1 = a0 + (a1 - a0) * ((i + 1) as f32 / n as f32);
        let p0x = cx + r * t0.cos();
        let p0y = cy + r * t0.sin();
        let p1x = cx + r * t1.cos();
        let p1y = cy + r * t1.sin();
        d = d.min(stroke(sd_segment(x, y, p0x, p0y, p1x, p1y), half));
    }
    d
}

// ---------------------------------------------------------------------------
// Glyph SDFs
// ---------------------------------------------------------------------------

const MIC_BALL_HY: f32 = 6.0;
const MIC_BALL_R: f32 = 3.3;
const MIC_BALL_CY: f32 = 11.8;
const MIC_ARC_CY: f32 = 14.9;
const MIC_ARC_R: f32 = 7.5;
const MIC_ARC_HALF: f32 = 1.3;
const MIC_STROKE: f32 = 1.3;

/// Classic microphone: capsule body, U-shaped bracket, stem, base bar.
fn sd_mic(x: f32, y: f32) -> f32 {
    let body = sd_round_box(x, y, 16.0, MIC_BALL_CY, MIC_BALL_R, MIC_BALL_HY, MIC_BALL_R);
    // Bracket opens upward: sweeps from lower-right, around the bottom, to
    // lower-left (screen angles 35°..145°, 90° = straight down).
    let d = 35.0_f32.to_radians();
    let a = 145.0_f32.to_radians();
    let bracket = sd_arc(x, y, 16.0, MIC_ARC_CY, MIC_ARC_R, d, a, MIC_ARC_HALF);
    let stem = stroke(
        sd_segment(x, y, 16.0, MIC_ARC_CY + MIC_ARC_R, 16.0, 25.2),
        MIC_STROKE,
    );
    let base = stroke(sd_segment(x, y, 12.6, 25.3, 19.4, 25.3), MIC_STROKE);
    body.min(bracket).min(stem).min(base)
}

/// Three dots (transcribing / "typing…" indicator).
fn sd_dots(x: f32, y: f32) -> f32 {
    [10.2_f32, 16.0, 21.8]
        .iter()
        .map(|&dx| sd_circle(x, y, dx, 16.2, 2.4))
        .fold(f32::INFINITY, f32::min)
}

/// Check mark.
fn sd_check(x: f32, y: f32) -> f32 {
    stroke(sd_segment(x, y, 10.2, 16.6, 14.3, 20.7), 1.75)
        .min(stroke(sd_segment(x, y, 14.3, 20.7, 21.9, 12.4), 1.75))
}

/// X mark.
fn sd_cross(x: f32, y: f32) -> f32 {
    stroke(sd_segment(x, y, 11.9, 11.9, 20.1, 20.1), 1.85)
        .min(stroke(sd_segment(x, y, 20.1, 11.9, 11.9, 20.1), 1.85))
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

type Rgba = [f32; 4]; // straight (non-premultiplied) alpha, 0..255

const fn rgba(r: u8, g: u8, b: u8) -> Rgba {
    [r as f32, g as f32, b as f32, 255.0]
}

const WHITE: Rgba = rgba(255, 255, 255);

/// Per-state gradient stops (top, bottom) sampled over the badge's vertical
/// extent. Vertical gradients give the flat badge an instant "material" feel.
fn badge_gradient(state: &UiState) -> (Rgba, Rgba) {
    match state {
        UiState::Idle => (rgba(158, 166, 181), rgba(93, 101, 119)), // slate
        UiState::Recording { .. } => (rgba(247, 94, 80), rgba(205, 32, 32)), // vivid red
        UiState::Transcribing { .. } => (rgba(255, 200, 92), rgba(236, 137, 12)), // amber
        UiState::Done { .. } => (rgba(88, 199, 142), rgba(28, 141, 88)), // green
        UiState::Error { .. } => (rgba(235, 64, 52), rgba(148, 16, 16)), // deep red
    }
}

// ---------------------------------------------------------------------------
// Rasterizer
// ---------------------------------------------------------------------------

const SS: u32 = 3; // supersampling factor per axis

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

    /// Source-over blend of one pixel.
    fn blend(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || (x as u32) >= self.size || (y as u32) >= self.size {
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

    /// Rasterize one layer: `sdf` describes the shape's coverage (negative
    /// inside), `paint` returns the color per design-space sample point.
    fn fill(&mut self, sdf: impl Fn(f32, f32) -> f32, paint: impl Fn(f32, f32) -> Rgba) {
        let s = self.size as i32;
        let scale = DESIGN / self.size as f32; // design units per pixel
        for py in 0..s {
            for px in 0..s {
                let mut acc = [0.0f32; 4];
                let mut any = false;
                for sy in 0..SS {
                    for sx in 0..SS {
                        let u = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                        let v = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                        let cov = (0.5 - sdf(u, v)).clamp(0.0, 1.0);
                        if cov > 0.0 {
                            any = true;
                            let c = paint(u, v);
                            acc[0] += c[0] * cov;
                            acc[1] += c[1] * cov;
                            acc[2] += c[2] * cov;
                            acc[3] += c[3] * cov;
                        }
                    }
                }
                if any {
                    let n = (SS * SS) as f32;
                    self.blend(
                        px,
                        py,
                        [
                            (acc[0] / n).round() as u8,
                            (acc[1] / n).round() as u8,
                            (acc[2] / n).round() as u8,
                            (acc[3] / n).round().min(255.0) as u8,
                        ],
                    );
                }
            }
        }
    }

    fn finish(self) -> IconData {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for p in self.pixels {
            rgba.extend_from_slice(&p);
        }
        IconData {
            width: self.size as i32,
            height: self.size as i32,
            rgba,
        }
    }
}

// ---------------------------------------------------------------------------
// Icon composition
// ---------------------------------------------------------------------------

const BADGE_R: f32 = 13.5;
const BADGE_C: f32 = 16.0;

fn badge_sdf(x: f32, y: f32) -> f32 {
    sd_circle(x, y, BADGE_C, BADGE_C, BADGE_R)
}

/// Render the icon for the given UI state. Deterministic and theme-independent.
pub fn icon_for_state(state: &UiState) -> IconData {
    let mut cv = Canvas::new();
    let (top, bottom) = badge_gradient(state);

    // Layer 1: badge with a vertical gradient (light at top, deep at bottom).
    cv.fill(badge_sdf, |_, y| {
        let t = ((y - (BADGE_C - BADGE_R)) / (2.0 * BADGE_R)).clamp(0.0, 1.0);
        [
            top[0] + (bottom[0] - top[0]) * t,
            top[1] + (bottom[1] - top[1]) * t,
            top[2] + (bottom[2] - top[2]) * t,
            255.0,
        ]
    });

    // Layer 2: soft gloss — white sheen fading out below the badge's equator.
    cv.fill(badge_sdf, |_, y| {
        let t = ((BADGE_C - y) / BADGE_R).clamp(0.0, 1.0);
        let a = 46.0 * t * t; // max ~18% alpha at the very top
        [255.0, 255.0, 255.0, a]
    });

    // Layer 3: subtle darker rim so the badge reads on any bar background.
    cv.fill(
        |x, y| stroke(badge_sdf(x, y), 0.5),
        |_, _| [0.0, 0.0, 0.0, 72.0],
    );

    // Layer 4: state glyph in white.
    let glyph = match state {
        UiState::Idle | UiState::Recording { .. } => sd_mic,
        UiState::Transcribing { .. } => sd_dots,
        UiState::Done { .. } => sd_check,
        UiState::Error { .. } => sd_cross,
    };
    cv.fill(glyph, |_, _| WHITE);

    cv.finish()
}

#[cfg(test)]
mod preview {
    //! Renders every state onto light & dark tray backgrounds and dumps a
    //! single BMP so the design can be eyeballed quickly:
    //!   cargo test --bin voice-type tray::icons::preview -- --nocapture
    use super::*;
    use std::time::Instant;

    fn write_bmp(path: &str, w: u32, h: u32, px: &[[u8; 3]]) {
        let row = (w * 3) as usize;
        let pad = (4 - row % 4) % 4;
        let mut data = Vec::with_capacity((row + pad) * h as usize);
        for y in (0..h).rev() {
            // BMP rows are bottom-up.
            for x in 0..w {
                let p = px[(y * w + x) as usize];
                data.extend_from_slice(&[p[2], p[1], p[0]]); // BGR
            }
            data.extend(std::iter::repeat_n(0, pad));
        }
        let header_size: u32 = 14 + 40;
        let file_len = header_size + data.len() as u32;
        let mut out = Vec::with_capacity(file_len as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_len.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&header_size.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 4]); // BI_RGB
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&data);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn render_previews() {
        let states = [
            UiState::Idle,
            UiState::Recording {
                started_at: Instant::now(),
            },
            UiState::Transcribing {
                started_at: Instant::now(),
            },
            UiState::Done {
                text: String::new(),
            },
            UiState::Error { msg: String::new() },
        ];
        let backgrounds: [[u8; 3]; 2] = [[243, 243, 246], [32, 32, 40]];
        let gap = 12u32;
        let w = states.len() as u32 * ICON_SIZE + (states.len() as u32 + 1) * gap;
        let h = 2 * ICON_SIZE + 3 * gap;
        let mut px = vec![[128u8, 128, 128]; (w * h) as usize];
        for (row, bg) in backgrounds.iter().enumerate() {
            for yy in 0..ICON_SIZE {
                for xx in 0..w {
                    px[((gap + row as u32 * (ICON_SIZE + gap) + yy) * w + xx) as usize] = *bg;
                }
            }
            for (i, st) in states.iter().enumerate() {
                let icon = icon_for_state(st);
                let ox = gap + i as u32 * (ICON_SIZE + gap);
                let oy = gap + row as u32 * (ICON_SIZE + gap);
                for yy in 0..ICON_SIZE {
                    for xx in 0..ICON_SIZE {
                        let p = ((yy * ICON_SIZE + xx) * 4) as usize;
                        let s: [u8; 4] = [
                            icon.rgba[p],
                            icon.rgba[p + 1],
                            icon.rgba[p + 2],
                            icon.rgba[p + 3],
                        ];
                        let d = &mut px[((oy + yy) * w + ox + xx) as usize];
                        let a = s[3] as f32 / 255.0;
                        for c in 0..3 {
                            d[c] = (s[c] as f32 * a + d[c] as f32 * (1.0 - a)).round() as u8;
                        }
                    }
                }
            }
        }
        let path = std::env::temp_dir().join("voice-type-tray-preview.bmp");
        write_bmp(path.to_str().unwrap(), w, h, &px);
        println!("preview written to {}", path.display());

        // ASCII preview: helps eyeball geometry without a GUI.
        let ramp = " .:-=+*#%@";
        let print_scaled = |icon: &IconData, step: u32, label: &str| {
            println!("--- {} {} ---", label, step);
            for yy in (0..ICON_SIZE).step_by(step as usize) {
                let mut line = String::new();
                for xx in (0..ICON_SIZE).step_by(step as usize) {
                    let (mut r_sum, mut g_sum, mut b_sum, mut a_sum, mut n) =
                        (0u32, 0u32, 0u32, 0u32, 0u32);
                    for dy in 0..step {
                        for dx in 0..step {
                            let p = (((yy + dy) * ICON_SIZE + xx + dx) * 4) as usize;
                            r_sum += icon.rgba[p] as u32;
                            g_sum += icon.rgba[p + 1] as u32;
                            b_sum += icon.rgba[p + 2] as u32;
                            a_sum += icon.rgba[p + 3] as u32;
                            n += 1;
                        }
                    }
                    let a = a_sum as f32 / n as f32 / 255.0;
                    if a < 0.05 {
                        line.push(' ');
                        continue;
                    }
                    // luminance of the composited color over mid-gray
                    let gray = 0.5 * 255.0;
                    let blend = |c: f32| c * a + gray * (1.0 - a);
                    let lum = 0.299 * blend(r_sum as f32 / n as f32)
                        + 0.587 * blend(g_sum as f32 / n as f32)
                        + 0.114 * blend(b_sum as f32 / n as f32);
                    let idx = ((lum / 255.0) * (ramp.len() - 1) as f32).round() as usize;
                    line.push(ramp.chars().nth(idx).unwrap());
                }
                println!("|{}|", line.trim_end());
            }
        };
        for st in &states {
            let icon = icon_for_state(st);
            print_scaled(&icon, 2, &format!("{:?}", st));
            print_scaled(&icon, 4, &format!("{:?} (16px tray size)", st));
        }
    }
}
