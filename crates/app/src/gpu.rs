//! Hardware acceleration: which graphics processor GPUI renders on.
//!
//! The renderer picks its device when the app starts, so the preference is
//! applied here, before GPUI initialises, and a change needs a restart.
//! * Windows: the per-app GPU preference (Settings > Display > Graphics),
//!   which orders the DXGI adapters GPUI enumerates.
//! * Linux: the PCI device GPUI should pick, or Mesa's software renderers.
//! * macOS: GPUI already uses the energy-efficient GPU; nothing to choose.

use std::sync::OnceLock;

use boxpilot_core::model::GpuPreference;

static STARTUP: OnceLock<GpuPreference> = OnceLock::new();

/// The preference the running renderer was started with.
pub fn startup_preference() -> GpuPreference {
    STARTUP.get().copied().unwrap_or_default()
}

/// Choices that have an effect on this platform (empty: nothing to choose).
pub fn choices() -> &'static [GpuPreference] {
    if cfg!(windows) {
        &[GpuPreference::Auto, GpuPreference::PowerSaving, GpuPreference::HighPerformance]
    } else if cfg!(target_os = "macos") {
        &[]
    } else {
        &[
            GpuPreference::Auto,
            GpuPreference::PowerSaving,
            GpuPreference::HighPerformance,
            GpuPreference::Software,
        ]
    }
}

/// Must run before GPUI creates its renderer (and before other threads start,
/// as it may set environment variables).
pub fn apply_at_startup(preference: GpuPreference) {
    let preference = if choices().contains(&preference) { preference } else { GpuPreference::Auto };
    let _ = STARTUP.set(preference);

    #[cfg(windows)]
    {
        let Ok(exe) = std::env::current_exe() else { return };
        let value = match preference {
            GpuPreference::PowerSaving => Some(1),
            GpuPreference::HighPerformance => Some(2),
            _ => None,
        };
        if let Err(e) = boxpilot_core::sys::set_gpu_preference(&exe.to_string_lossy(), value) {
            log::warn!("could not set the GPU preference: {e:#}");
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    // SAFETY: called at the top of `main`, before any other thread exists.
    unsafe {
        match preference {
            GpuPreference::Software => {
                // Mesa: llvmpipe for GL and lavapipe for Vulkan.
                std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
                if let Some(icd) = lavapipe_icd() {
                    std::env::set_var("VK_ICD_FILENAMES", icd);
                }
            }
            GpuPreference::PowerSaving | GpuPreference::HighPerformance => {
                let want_integrated = preference == GpuPreference::PowerSaving;
                if let Some(device) = pick_drm_device(want_integrated) {
                    std::env::set_var("ZED_DEVICE_ID", format!("{device:#06x}"));
                }
                if !want_integrated {
                    std::env::set_var("DRI_PRIME", "1");
                }
            }
            GpuPreference::Auto => {}
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn lavapipe_icd() -> Option<String> {
    ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d"]
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("lvp_icd")))
        .map(|p| p.display().to_string())
}

/// A graphics card: PCI vendor and device ids, and whether it drives the
/// boot display (`boot_vga`).
#[cfg(all(unix, not(target_os = "macos")))]
struct Card {
    vendor: u32,
    device: u32,
    boot: bool,
}

#[cfg(all(unix, not(target_os = "macos")))]
fn drm_cards() -> Vec<Card> {
    let read = |p: std::path::PathBuf| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let hex = |s: String| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok();
    let mut cards = Vec::new();
    for entry in std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let dev = entry.path().join("device");
        let (Some(vendor), Some(device)) = (read(dev.join("vendor")).and_then(hex), read(dev.join("device")).and_then(hex)) else {
            continue;
        };
        cards.push(Card { vendor, device, boot: read(dev.join("boot_vga")).as_deref() == Some("1") });
    }
    cards
}

/// On hybrid systems the GPU driving the boot display (`boot_vga`) is the
/// integrated one; any other GPU is treated as the discrete one.
#[cfg(all(unix, not(target_os = "macos")))]
fn pick_drm_device(integrated: bool) -> Option<u32> {
    drm_cards().iter().find(|c| c.boot == integrated).map(|c| c.device)
}

/// Hybrid graphics on Linux: when the GPU driving the display has no Vulkan
/// driver, GPUI renders on the other one and every frame is copied across,
/// which makes scrolling stutter. Returns that GPU's maker and the packages
/// with its Mesa driver.
pub fn missing_display_driver() -> Option<(&'static str, &'static str)> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        static MISSING: std::sync::LazyLock<Option<(&'static str, &'static str)>> = std::sync::LazyLock::new(|| {
            let cards = drm_cards();
            let display = cards.iter().find(|c| c.boot).filter(|_| cards.len() > 1)?;
            let (maker, icd, packages) = match display.vendor {
                0x8086 => ("Intel", "intel", "vulkan-intel (Arch) or mesa-vulkan-drivers (Debian, Ubuntu, Fedora)"),
                0x1002 => ("AMD", "radeon", "vulkan-radeon (Arch) or mesa-vulkan-drivers (Debian, Ubuntu, Fedora)"),
                _ => return None,
            };
            let installed = ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d"]
                .iter()
                .filter_map(|dir| std::fs::read_dir(dir).ok())
                .flatten()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(icd));
            (!installed).then_some((maker, packages))
        });
        *MISSING
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    None
}
