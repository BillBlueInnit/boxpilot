//! Writes the macOS app icon from the same drawing as the .exe icon:
//! `cargo run -p boxpilot --example icns -- BoxPilot.icns`

#[allow(dead_code)]
mod icon_art {
    include!("../src/icon_art.rs");
}

use std::io::Write as _;

/// A PNG of straight RGBA pixels.
fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    let mut raw = Vec::with_capacity(rgba.len() + size as usize);
    for row in rgba.chunks(size as usize * 4) {
        raw.push(0); // no filter
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&raw).unwrap();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z.finish().unwrap());
    chunk(&mut out, b"IEND", &[]);
    out
}

/// The icon on macOS's grid: the shape fills about 80 % of the canvas.
fn padded(size: u32) -> Vec<u8> {
    let body = (size as f32 * 824.0 / 1024.0).round() as u32;
    let art = icon_art::render_icon(body, true);
    let off = (size - body) / 2;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..body {
        let src = (y * body * 4) as usize;
        let dst = (((y + off) * size + off) * 4) as usize;
        out[dst..dst + body as usize * 4].copy_from_slice(&art[src..src + body as usize * 4]);
    }
    out
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "BoxPilot.icns".into());
    let entries: [(&[u8; 4], u32); 10] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"ic11", 32),
        (b"icp6", 64),
        (b"ic12", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic13", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
    ];
    let mut body = Vec::new();
    for (kind, size) in entries {
        let data = png(size, &padded(size));
        body.extend_from_slice(kind);
        body.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
        body.extend_from_slice(&data);
    }
    let mut icns = b"icns".to_vec();
    icns.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    icns.extend_from_slice(&body);
    std::fs::write(&path, icns).expect("write icns");
    println!("{path}");
}
