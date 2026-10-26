#[allow(dead_code)]
mod icon_art {
    include!("src/icon_art.rs");
}

/// Writes a multi-resolution .ico with 32-bit BMP entries.
fn write_ico(path: &std::path::Path) -> std::io::Result<()> {
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let rgba = icon_art::render_icon(s, true);
            let mut dib = Vec::new();
            // BITMAPINFOHEADER; height is doubled to include the AND mask.
            dib.extend_from_slice(&40u32.to_le_bytes());
            dib.extend_from_slice(&(s as i32).to_le_bytes());
            dib.extend_from_slice(&((s * 2) as i32).to_le_bytes());
            dib.extend_from_slice(&1u16.to_le_bytes());
            dib.extend_from_slice(&32u16.to_le_bytes());
            dib.extend_from_slice(&[0u8; 24]);
            for y in (0..s).rev() {
                for x in 0..s {
                    let i = ((y * s + x) * 4) as usize;
                    dib.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
                }
            }
            let mask_row = ((s + 31) / 32 * 4) as usize;
            dib.extend(std::iter::repeat_n(0u8, mask_row * s as usize));
            dib
        })
        .collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in &images {
        ico.extend_from_slice(img);
    }
    std::fs::write(path, ico)
}

fn main() {
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.ico");
    write_ico(&out).expect("write icon");
    let mut res = winresource::WindowsResource::new();
    res.set_icon_with_id(out.to_str().unwrap(), "1");
    res.set("FileDescription", "BoxPilot");
    res.set("ProductName", "BoxPilot");
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed icon: {e}");
    }
}
