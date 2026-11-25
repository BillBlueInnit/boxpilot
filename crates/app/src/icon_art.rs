// Procedurally drawn application icon, shared by build.rs (exe icon) and the
// tray. Kept dependency-free so it can be `include!`d from the build script.

/// The same icon as SVG for the window (rounded square inset 4 %, radius 23 %,
/// diagonal blue-to-violet gradient, white paper plane). Keep the geometry in
/// step with `render_icon`.
pub const SVG: &str = concat!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">"##,
    r##"<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">"##,
    r##"<stop offset="0" stop-color="#3b82f6"/><stop offset="1" stop-color="#8b5cf6"/></linearGradient></defs>"##,
    r##"<rect x="2.56" y="2.56" width="58.88" height="58.88" rx="14.72" fill="url(#g)"/>"##,
    r##"<polygon points="12.8,32 51.2,14.08 37.12,51.2 29.44,36.48" fill="#ffffff"/>"##,
    r##"</svg>"##,
);

/// Renders the icon as straight RGBA. `active` selects the colored variant;
/// the inactive one is grey, used by the tray while the core is stopped.
pub fn render_icon(size: u32, active: bool) -> Vec<u8> {
    const SS: u32 = 4;
    let plane: [(f32, f32); 4] = [(0.20, 0.50), (0.80, 0.22), (0.58, 0.80), (0.46, 0.57)];
    let (c0, c1) = if active {
        ((0x3b, 0x82, 0xf6), (0x8b, 0x5c, 0xf6))
    } else {
        ((0x8a, 0x8f, 0x98), (0x5f, 0x64, 0x6d))
    };
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (mut bg, mut fg) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if in_rounded_rect(u, v, 0.04, 0.23) {
                        bg += 1;
                        if in_polygon(u, v, &plane) {
                            fg += 1;
                        }
                    }
                }
            }
            let total = (SS * SS) as f32;
            let t = ((x + y) as f32 / (2 * size) as f32).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| a as f32 + (b as f32 - a as f32) * t;
            let base = [mix(c0.0, c1.0), mix(c0.1, c1.1), mix(c0.2, c1.2)];
            let f = fg as f32 / bg.max(1) as f32;
            let i = ((y * size + x) * 4) as usize;
            for c in 0..3 {
                out[i + c] = (base[c] + (255.0 - base[c]) * f).round() as u8;
            }
            out[i + 3] = (bg as f32 / total * 255.0).round() as u8;
        }
    }
    out
}

/// Renders a macOS menu-bar template icon (monochrome alpha mask, pure black
/// RGB so AppKit tints it natively for macOS 26/27 Liquid Glass menu bars).
///
/// Mirrors SF Symbols `paperplane.fill` when `active` (core running) and a
/// crisp hollow outlined `paperplane` when inactive (core stopped), sized to
/// Apple's ~15.5 pt optical footprint inside a 22 pt Retina (`44×44` px) box.
#[allow(dead_code)]
pub fn render_macos_tray_icon(size: u32, active: bool) -> Vec<u8> {
    const SS: u32 = 8;
    // Signature BoxPilot dart silhouette, optically centered inside the 22pt bar.
    let plane: [(f32, f32); 4] = [(0.13, 0.49), (0.86, 0.16), (0.59, 0.86), (0.46, 0.56)];
    let stroke_half_w = 0.036_f32;
    let total = (SS * SS) as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let mut cov = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    let hit = if active {
                        in_polygon(u, v, &plane)
                    } else {
                        near_polygon_edge(u, v, &plane, stroke_half_w)
                    };
                    if hit {
                        cov += 1;
                    }
                }
            }
            let i = ((y * size + x) * 4) as usize;
            // Template images use only the alpha channel; RGB stays 0.
            out[i + 3] = (cov as f32 / total * 255.0).round() as u8;
        }
    }
    out
}

#[allow(dead_code)]
fn near_polygon_edge(x: f32, y: f32, pts: &[(f32, f32)], half_w: f32) -> bool {
    let r2 = half_w * half_w;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (ax, ay) = pts[j];
        let (bx, by) = pts[i];
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((x - ax) * dx + (y - ay) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (px, py) = (ax + dx * t, ay + dy * t);
        let d2 = (x - px).powi(2) + (y - py).powi(2);
        if d2 <= r2 {
            return true;
        }
        j = i;
    }
    false
}

fn in_rounded_rect(u: f32, v: f32, inset: f32, radius: f32) -> bool {
    let (lo, hi) = (inset, 1.0 - inset);
    if u < lo || u > hi || v < lo || v > hi {
        return false;
    }
    let cx = u.clamp(lo + radius, hi - radius);
    let cy = v.clamp(lo + radius, hi - radius);
    (u - cx).powi(2) + (v - cy).powi(2) <= radius * radius
}

fn in_polygon(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}
