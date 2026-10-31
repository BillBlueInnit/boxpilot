//! Writes the Linux app icons (the freedesktop hicolor theme) from the same
//! drawing as the .exe and macOS icons:
//! `cargo run -p boxpilot --example linux_icons -- <dir>` creates
//! `<dir>/hicolor/<n>x<n>/apps/boxpilot.png` for the usual sizes.

#[allow(dead_code)]
mod icon_art {
    include!("../src/icon_art.rs");
}

#[path = "common/png.rs"]
mod png;

use png::{padded, png};

fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "icons".into()));
    for size in [16u32, 24, 32, 48, 64, 128, 256, 512] {
        // Small sizes use the whole canvas so the shape stays legible; larger
        // ones get the same margin as on macOS, like other desktop app icons.
        let rgba = if size <= 32 { icon_art::render_icon(size, true) } else { padded(size) };
        let dir = root.join(format!("hicolor/{size}x{size}/apps"));
        std::fs::create_dir_all(&dir).expect("create icon dir");
        let path = dir.join("boxpilot.png");
        std::fs::write(&path, png(size, &rgba)).expect("write icon");
        println!("{}", path.display());
    }
}
