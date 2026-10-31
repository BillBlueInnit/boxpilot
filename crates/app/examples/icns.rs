//! Writes the macOS app icon from the same drawing as the .exe icon:
//! `cargo run -p boxpilot --example icns -- BoxPilot.icns`

#[allow(dead_code)]
mod icon_art {
    include!("../src/icon_art.rs");
}

#[path = "common/png.rs"]
mod png;

use png::{padded, png};

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
