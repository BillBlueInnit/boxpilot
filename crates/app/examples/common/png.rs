//! Shared by the icon examples: a PNG encoder and the padded app icon.

use std::io::Write as _;

/// A PNG of straight RGBA pixels.
pub fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
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
pub fn padded(size: u32) -> Vec<u8> {
    let body = (size as f32 * 824.0 / 1024.0).round() as u32;
    let art = super::icon_art::render_icon(body, true);
    let off = (size - body) / 2;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..body {
        let src = (y * body * 4) as usize;
        let dst = (((y + off) * size + off) * 4) as usize;
        out[dst..dst + body as usize * 4].copy_from_slice(&art[src..src + body as usize * 4]);
    }
    out
}
