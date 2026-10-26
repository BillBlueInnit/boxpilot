// Procedurally drawn application icon, shared by build.rs (exe icon) and the
// tray. Kept dependency-free so it can be `include!`d from the build script.

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
