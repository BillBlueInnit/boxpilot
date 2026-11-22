//! Adaptive sizing.
//!
//! * DPI: GPUI renders in logical pixels at each monitor's scale factor and
//!   re-renders when the window moves to a monitor with different scaling.
//! * Interface size: everything BoxPilot sizes itself is written in [`dp`]
//!   (pixels at 100 %), multiplied by the interface scale; component controls
//!   follow through the rem size. The scale follows the OS text-size setting
//!   unless the user picks one.
//! * Window size: the first window fits the display it opens on, and the
//!   size, position and maximized state are remembered between sessions.
//! * Layout: breakpoints on the window width (in `dp`) collapse the sidebar to
//!   icons, tighten spacing and drop table columns as space runs out.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use boxpilot_core::model::{Language, SavedWindow, Settings, UiFont};
use gpui_kit::component::Theme;
use gpui_kit::{App, Bounds, FontFallbacks, Pixels, SharedString, Size, Window, WindowBounds, point, px, size};

const BASE_FONT: f32 = 16.0;

/// Current interface scale as f32 bits (1.0 = 100 %).
static SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000);
/// The OS text-size factor, read at start-up and when the window regains focus.
static SYSTEM_SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000);
/// Opacity of page sheets and cards: below 1 when the window is translucent.
static SURFACE_ALPHA: AtomicU32 = AtomicU32::new(0x3F80_0000);

pub fn surface_alpha() -> f32 {
    f32::from_bits(SURFACE_ALPHA.load(Ordering::Relaxed))
}

/// Whether the window actually shows a backdrop (set with the material, so
/// drawing follows what was applied, not just the setting).
static TRANSLUCENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether the active backdrop is Acrylic / OS Blur (as opposed to Mica / Opaque).
static GLASS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn translucent() -> bool {
    TRANSLUCENT.load(Ordering::Relaxed)
}

pub fn is_glass() -> bool {
    GLASS.load(Ordering::Relaxed)
}

pub fn set_surface_alpha(translucent: bool, glass: bool, transparency: u32) -> bool {
    TRANSLUCENT.store(translucent, Ordering::Relaxed);
    GLASS.store(translucent && glass, Ordering::Relaxed);
    // Never fully clear: text needs something to sit on.
    let a = if translucent { 1.0 - transparency.min(80) as f32 / 100.0 } else { 1.0 };
    let changed = (a - surface_alpha()).abs() > f32::EPSILON;
    SURFACE_ALPHA.store(a.to_bits(), Ordering::Relaxed);
    changed
}

pub fn scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

/// A length given in pixels at 100 % interface size.
pub fn dp(v: f32) -> Pixels {
    px(v * scale())
}

/// Interface sizes offered in Settings (percent; 0 = follow the system).
pub const SCALES: [u32; 7] = [0, 85, 100, 115, 125, 150, 175];

pub fn refresh_system_scale() {
    let s = boxpilot_core::sys::system_text_scale();
    SYSTEM_SCALE.store(s.to_bits(), Ordering::Relaxed);
}

pub fn system_scale() -> f32 {
    f32::from_bits(SYSTEM_SCALE.load(Ordering::Relaxed))
}

fn wanted_scale(settings: &Settings) -> f32 {
    if settings.ui_scale == 0 {
        system_scale()
    } else {
        settings.ui_scale.clamp(50, 300) as f32 / 100.0
    }
}

/// Applies the interface scale and UI font; returns whether either changed.
pub fn apply_scale(settings: &Settings, cx: &mut App) -> bool {
    let s = wanted_scale(settings);
    let scale_changed = (s - scale()).abs() > f32::EPSILON;
    SCALE.store(s.to_bits(), Ordering::Relaxed);
    // Component controls are sized in rems; the root derives rem from this.
    let theme = Theme::global_mut(cx);
    theme.font_size = px(BASE_FONT * s);
    theme.mono_font_size = px(13.0 * s);
    let font_changed = apply_fonts(settings, cx);
    scale_changed || font_changed
}

/// Bundled Google Sans variable font (SIL Open Font License 1.1, `assets/fonts/OFL.txt`).
const GOOGLE_SANS_TTF: &[u8] = include_bytes!("../../assets/fonts/GoogleSans.ttf");
static GOOGLE_SANS_LOADED: AtomicBool = AtomicBool::new(false);
static DEFAULT_UI_FONT: OnceLock<SharedString> = OnceLock::new();

/// Registers bundled open-source fonts before the first window opens.
pub fn register_bundled_fonts(cx: &mut App) {
    let _ = DEFAULT_UI_FONT.get_or_init(|| Theme::global(cx).font_family.clone());
    if GOOGLE_SANS_LOADED.load(Ordering::Relaxed) {
        return;
    }
    match cx.text_system().add_fonts(vec![Cow::Borrowed(GOOGLE_SANS_TTF)]) {
        Ok(()) => {
            GOOGLE_SANS_LOADED.store(true, Ordering::Relaxed);
            log::info!("registered bundled Google Sans font");
        }
        Err(e) => log::warn!("could not register bundled Google Sans font: {e:#}"),
    }
}

fn installed_fonts(cx: &App) -> &'static HashSet<String> {
    static FONTS: OnceLock<HashSet<String>> = OnceLock::new();
    FONTS.get_or_init(|| cx.text_system().all_font_names().into_iter().collect())
}

fn pick_installed(candidates: &[&'static str], cx: &App) -> Option<&'static str> {
    let all = installed_fonts(cx);
    candidates.iter().copied().find(|name| all.contains(*name))
}

const SOURCE_HAN_SANS_SC: &[&str] = &[
    "Source Han Sans SC",
    "Source Han Sans CN",
    "思源黑体 CN",
    "思源黑体",
    "Noto Sans SC",
    "Noto Sans CJK SC",
];

const SOURCE_HAN_SANS_TC: &[&str] = &[
    "Source Han Sans TC",
    "Source Han Sans TW",
    "思源黑體 TW",
    "思源黑體",
    "Noto Sans TC",
    "Noto Sans CJK TC",
];

fn pick_source_han_sans(lang: Language, cx: &App) -> Option<&'static str> {
    let resolved = match lang {
        Language::System => crate::i18n::active_language(),
        other => other,
    };
    if resolved == Language::TraditionalChinese {
        pick_installed(SOURCE_HAN_SANS_TC, cx).or_else(|| pick_installed(SOURCE_HAN_SANS_SC, cx))
    } else {
        pick_installed(SOURCE_HAN_SANS_SC, cx).or_else(|| pick_installed(SOURCE_HAN_SANS_TC, cx))
    }
}

/// Whether Source Han Sans / Noto Sans CJK (思源黑体) is installed on this system.
pub fn has_source_han_sans(lang: Language, cx: &App) -> bool {
    pick_source_han_sans(lang, cx).is_some()
}

#[cfg(windows)]
fn normalized_noto_cjk(family: &str, cx: &mut App) -> Option<SharedString> {
    let (file_name, alias) = match family {
        "Noto Sans SC" => ("NotoSansSC-VF.ttf", "BoxPilot CJK"),
        "Noto Sans TC" => ("NotoSansTC-VF.ttf", "BoxPilot CJT"),
        _ => return None,
    };
    static SC_ALIAS: OnceLock<Option<SharedString>> = OnceLock::new();
    static TC_ALIAS: OnceLock<Option<SharedString>> = OnceLock::new();
    let slot = if family == "Noto Sans SC" { &SC_ALIAS } else { &TC_ALIAS };
    slot.get_or_init(|| {
        let mut candidates = Vec::new();
        if let Ok(win) = std::env::var("WINDIR") {
            candidates.push(std::path::PathBuf::from(win).join("Fonts").join(file_name));
        } else {
            candidates.push(std::path::PathBuf::from(r"C:\Windows\Fonts").join(file_name));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(std::path::PathBuf::from(local).join(r"Microsoft\Windows\Fonts").join(file_name));
        }
        let mut bytes = candidates.into_iter().find_map(|p| std::fs::read(p).ok())?;
        if !normalize_sfnt_for_ui(&mut bytes, family, alias) {
            return None;
        }
        match cx.text_system().add_fonts(vec![Cow::Owned(bytes)]) {
            Ok(()) => {
                log::info!("registered vertically-normalized {family} as {alias}");
                Some(SharedString::from(alias))
            }
            Err(e) => {
                log::warn!("could not register normalized {family}: {e:#}");
                None
            }
        }
    })
    .clone()
}

/// Normalizes an SFNT (`ttf`/`otf`) font's vertical metrics (`hhea`, `OS/2`
/// `sTypo*` and `usWin*`, `USE_TYPO_METRICS`) to the same optical baseline as
/// `Microsoft YaHei UI` and bundled `Google Sans` (`0.968 / -0.241 em`, midpoint
/// `0.3635 em`) and aliases `old_family` → `new_family` in-place (must be same
/// ASCII byte length) so DirectWrite picks up the normalized copy cleanly.
#[cfg(any(windows, test))]
fn normalize_sfnt_for_ui(data: &mut [u8], old_family: &str, new_family: &str) -> bool {
    if data.len() < 12 || old_family.len() != new_family.len() || !old_family.is_ascii() || !new_family.is_ascii() {
        return false;
    }
    let sfnt = &data[0..4];
    if sfnt != b"\x00\x01\x00\x00" && sfnt != b"OTTO" && sfnt != b"true" {
        return false;
    }
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    if data.len() < 12 + num_tables * 16 {
        return false;
    }

    let table_checksum = |buf: &[u8]| -> u32 {
        let mut sum = 0u32;
        let mut chunks = buf.chunks_exact(4);
        for c in &mut chunks {
            sum = sum.wrapping_add(u32::from_be_bytes([c[0], c[1], c[2], c[3]]));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut tail = [0u8; 4];
            tail[..rem.len()].copy_from_slice(rem);
            sum = sum.wrapping_add(u32::from_be_bytes(tail));
        }
        sum
    };

    let mut head = None;
    let mut hhea = None;
    let mut os2 = None;
    let mut name = None;
    for i in 0..num_tables {
        let rec = 12 + i * 16;
        let tag = &data[rec..rec + 4];
        let off = u32::from_be_bytes([data[rec + 8], data[rec + 9], data[rec + 10], data[rec + 11]]) as usize;
        let len = u32::from_be_bytes([data[rec + 12], data[rec + 13], data[rec + 14], data[rec + 15]]) as usize;
        if off.checked_add(len).is_none_or(|end| end > data.len()) {
            return false;
        }
        match tag {
            b"head" => head = Some((rec, off, len)),
            b"hhea" => hhea = Some((rec, off, len)),
            b"OS/2" => os2 = Some((rec, off, len)),
            b"name" => name = Some((rec, off, len)),
            _ => {}
        }
    }
    let (Some((head_rec, head_off, head_len)), Some((hhea_rec, hhea_off, hhea_len)), Some((os2_rec, os2_off, os2_len)), Some((name_rec, name_off, name_len))) =
        (head, hhea, os2, name)
    else {
        return false;
    };
    if head_len < 20 || hhea_len < 10 || os2_len < 78 || name_len < 6 {
        return false;
    }

    let upem = u16::from_be_bytes([data[head_off + 18], data[head_off + 19]]) as f32;
    if upem < 64.0 {
        return false;
    }
    let asc = (0.968 * upem).round() as i16;
    let desc = (0.241 * upem).round() as i16;

    // hhea: ascent, descent, lineGap
    data[hhea_off + 4..hhea_off + 6].copy_from_slice(&asc.to_be_bytes());
    data[hhea_off + 6..hhea_off + 8].copy_from_slice(&(-desc).to_be_bytes());
    data[hhea_off + 8..hhea_off + 10].copy_from_slice(&0i16.to_be_bytes());
    let hhea_csum = table_checksum(&data[hhea_off..hhea_off + hhea_len]);
    data[hhea_rec + 4..hhea_rec + 8].copy_from_slice(&hhea_csum.to_be_bytes());

    // OS/2: USE_TYPO_METRICS (bit 7), sTypoAscender/Descender/LineGap, usWinAscent/Descent
    let fs_sel = u16::from_be_bytes([data[os2_off + 62], data[os2_off + 63]]) | 0x0080;
    data[os2_off + 62..os2_off + 64].copy_from_slice(&fs_sel.to_be_bytes());
    data[os2_off + 68..os2_off + 70].copy_from_slice(&asc.to_be_bytes());
    data[os2_off + 70..os2_off + 72].copy_from_slice(&(-desc).to_be_bytes());
    data[os2_off + 72..os2_off + 74].copy_from_slice(&0i16.to_be_bytes());
    data[os2_off + 74..os2_off + 76].copy_from_slice(&(asc as u16).to_be_bytes());
    data[os2_off + 76..os2_off + 78].copy_from_slice(&(desc as u16).to_be_bytes());
    let os2_csum = table_checksum(&data[os2_off..os2_off + os2_len]);
    data[os2_rec + 4..os2_rec + 8].copy_from_slice(&os2_csum.to_be_bytes());

    // name: replace family strings in-place (UTF-16BE for platform 3/0, ASCII for platform 1)
    let count = u16::from_be_bytes([data[name_off + 2], data[name_off + 3]]) as usize;
    let str_base = name_off + u16::from_be_bytes([data[name_off + 4], data[name_off + 5]]) as usize;
    let old_u16: Vec<u8> = old_family.bytes().flat_map(|b| [0u8, b]).collect();
    let new_u16: Vec<u8> = new_family.bytes().flat_map(|b| [0u8, b]).collect();
    let mut renamed = false;
    for i in 0..count {
        let rec = name_off + 6 + i * 12;
        if rec + 12 > name_off + name_len {
            break;
        }
        let platform = u16::from_be_bytes([data[rec], data[rec + 1]]);
        let name_id = u16::from_be_bytes([data[rec + 6], data[rec + 7]]);
        let slen = u16::from_be_bytes([data[rec + 8], data[rec + 9]]) as usize;
        let soff = u16::from_be_bytes([data[rec + 10], data[rec + 11]]) as usize;
        if !matches!(name_id, 1 | 4 | 16) {
            continue;
        }
        let start = str_base + soff;
        let Some(end) = start.checked_add(slen) else { continue };
        if end > name_off + name_len {
            continue;
        }
        if platform == 1 && &data[start..end] == old_family.as_bytes() {
            data[start..end].copy_from_slice(new_family.as_bytes());
            renamed = true;
        } else if (platform == 3 || platform == 0) && &data[start..end] == old_u16.as_slice() {
            data[start..end].copy_from_slice(&new_u16);
            renamed = true;
        }
    }
    if !renamed {
        return false;
    }
    let name_csum = table_checksum(&data[name_off..name_off + name_len]);
    data[name_rec + 4..name_rec + 8].copy_from_slice(&name_csum.to_be_bytes());

    // head: zero checkSumAdjustment, update head checksum, then write whole-file adjustment
    data[head_off + 8..head_off + 12].copy_from_slice(&0u32.to_be_bytes());
    let head_csum = table_checksum(&data[head_off..head_off + head_len]);
    data[head_rec + 4..head_rec + 8].copy_from_slice(&head_csum.to_be_bytes());
    let adj = 0xB1B0_AFBAu32.wrapping_sub(table_checksum(data));
    data[head_off + 8..head_off + 12].copy_from_slice(&adj.to_be_bytes());
    true
}

/// Resolves a `UiFont` choice to the concrete font family name for `lang`,
/// lazily registering any vertically-normalized copy on Windows when needed.
pub fn resolve_ui_font(ui_font: UiFont, lang: Language, cx: &mut App) -> SharedString {
    #[cfg(windows)]
    let resolved_lang = match lang {
        Language::System => crate::i18n::active_language(),
        other => other,
    };
    // On Windows, Segoe UI Variable and Windows 11's bundled NotoSansSC-VF both
    // leave OS/2 USE_TYPO_METRICS unset with tall winAscent (1.08–1.16 em), which
    // pushes CJK glyphs ~2 px below vertical center inside buttons and pills.
    // Microsoft YaHei UI / JhengHei UI set USE_TYPO_METRICS with balanced ascent/descent
    // (0.968 / 0.241 em) so Chinese UI controls sit dead-center on their midline.
    #[cfg(windows)]
    let system_ui_candidates: &[&str] = match resolved_lang {
        Language::SimplifiedChinese => &["Microsoft YaHei UI", "Segoe UI Variable", "Segoe UI"],
        Language::TraditionalChinese => &["Microsoft JhengHei UI", "Microsoft YaHei UI", "Segoe UI Variable", "Segoe UI"],
        _ => &["Segoe UI Variable", "Segoe UI Variable Text", "Segoe UI"],
    };
    #[cfg(not(windows))]
    let system_ui_candidates: &[&str] = &[];

    let system_ui = || -> SharedString {
        pick_installed(system_ui_candidates, cx)
            .map(SharedString::from)
            .or_else(|| DEFAULT_UI_FONT.get().cloned())
            .unwrap_or_else(|| ".SystemUIFont".into())
    };

    match ui_font {
        UiFont::System => system_ui(),
        UiFont::GoogleSans => {
            if GOOGLE_SANS_LOADED.load(Ordering::Relaxed) {
                "Google Sans".into()
            } else {
                system_ui()
            }
        }
        UiFont::SourceHanSans => match pick_source_han_sans(lang, cx) {
            #[cfg(windows)]
            Some(name) => normalized_noto_cjk(name, cx).unwrap_or_else(|| SharedString::from(name)),
            #[cfg(not(windows))]
            Some(name) => SharedString::from(name),
            None => system_ui(),
        },
    }
}

/// Applies the user's chosen UI typeface and monospace font; returns whether the
/// active UI font family changed (for live refresh without restart).
pub fn apply_fonts(settings: &Settings, cx: &mut App) -> bool {
    #[cfg(windows)]
    let mono_candidates: &[&str] = &["Cascadia Mono", "Cascadia Code", "Consolas"];
    #[cfg(target_os = "macos")]
    let mono_candidates: &[&str] = &["SF Mono", "Menlo"];
    #[cfg(all(unix, not(target_os = "macos")))]
    let mono_candidates: &[&str] = &["JetBrains Mono", "Noto Sans Mono", "DejaVu Sans Mono"];

    let wanted_ui = resolve_ui_font(settings.ui_font, settings.language, cx);
    let mono = pick_installed(mono_candidates, cx);
    let theme = Theme::global_mut(cx);
    let changed = theme.font_family != wanted_ui;
    theme.font_family = wanted_ui;
    if let Some(name) = mono {
        theme.mono_font_family = name.into();
    }
    changed
}

/// Explicit fallback chain for CJK & symbols so DirectWrite / FreeType never
/// drop back to jagged legacy bitmap/GDI fonts or top-heavy metrics that shift
/// CJK labels below vertical center inside buttons and pills.
pub fn font_fallbacks(lang: Language, cx: &App) -> Option<FontFallbacks> {
    #[cfg(windows)]
    {
        let resolved = match lang {
            Language::System => crate::i18n::active_language(),
            other => other,
        };
        // Prefer Microsoft YaHei UI / JhengHei UI first because their balanced
        // OS/2 typo metrics (0.968 / 0.241 em) align identically with Google Sans
        // and icons inside buttons; Windows 11's NotoSansSC-VF has winAscent=1.16 em
        // which shifts fallback CJK text ~2.5 px downward.
        let list: &[&str] = match resolved {
            Language::SimplifiedChinese => &[
                "Microsoft YaHei UI",
                "Source Han Sans SC",
                "Source Han Sans CN",
                "思源黑体 CN",
                "HarmonyOS Sans SC",
                "MiSans",
                "Noto Sans SC",
                "Noto Sans CJK SC",
                "Segoe UI",
                "Segoe UI Symbol",
                "Segoe UI Emoji",
            ],
            Language::TraditionalChinese => &[
                "Microsoft JhengHei UI",
                "Microsoft YaHei UI",
                "Source Han Sans TC",
                "Source Han Sans TW",
                "思源黑體 TW",
                "Noto Sans TC",
                "Noto Sans CJK TC",
                "Segoe UI",
                "Segoe UI Symbol",
                "Segoe UI Emoji",
            ],
            _ => &[
                "Microsoft YaHei UI",
                "Source Han Sans SC",
                "Source Han Sans CN",
                "Noto Sans SC",
                "Noto Sans CJK SC",
                "Segoe UI",
                "Segoe UI Symbol",
                "Segoe UI Emoji",
            ],
        };
        let all = installed_fonts(cx);
        let picked: Vec<String> = list
            .iter()
            .filter(|name| all.contains(**name))
            .map(|name| (*name).to_string())
            .collect();
        (!picked.is_empty()).then(|| FontFallbacks::from_fonts(picked))
    }
    #[cfg(not(windows))]
    {
        let _ = (lang, cx);
        None
    }
}

/// Layout decisions for the current window width.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Sidebar collapsed to icons.
    pub compact_nav: bool,
    /// Tight spacing for small windows.
    pub narrow: bool,
    pub sidebar: f32,
    /// Width available to page content, in `dp`.
    pub content: f32,
    /// Left/right page inset: the padding plus, on very wide windows, the
    /// margin that centers the capped content column.
    pub inset: f32,
}

/// Page content stops growing here; wider windows center it.
pub const MAX_CONTENT: f32 = 1600.0;

pub fn metrics(window: &Window) -> Metrics {
    let width = window.viewport_size().width.as_f32() / scale();
    let compact_nav = width < 1000.0;
    let narrow = width < 820.0;
    let sidebar = if compact_nav { 64.0 } else { 216.0 };
    let pad = if narrow { 16.0 } else { 24.0 };
    let available = width - sidebar - pad * 2.0;
    let content = available.clamp(240.0, MAX_CONTENT);
    let inset = pad + (available - MAX_CONTENT).max(0.0) / 2.0;
    Metrics { compact_nav, narrow, sidebar, content, inset }
}

/// How many columns of at least `min` (plus `gap` between them) fit in
/// `content`. Lay them out with a CSS grid so they share the real width:
/// fixed card widths computed from this estimate overflow by a pixel and wrap.
pub fn grid_columns(content: f32, min: f32, gap: f32) -> u16 {
    ((content + gap) / (min + gap)).floor().clamp(1.0, 12.0) as u16
}

/// The window's opening bounds: the saved ones when they still fit a
/// connected display, otherwise a size proportional to the primary display.
pub fn initial_bounds(settings: &Settings, cx: &App) -> (WindowBounds, Size<Pixels>) {
    let visible = cx
        .primary_display()
        .map(|d| d.visible_bounds())
        .unwrap_or_else(|| Bounds::new(point(px(0.), px(0.)), size(px(1366.), px(768.))));
    let (vw, vh) = (visible.size.width.as_f32(), visible.size.height.as_f32());
    let min = size(px(860f32.min(vw * 0.9)), px(560f32.min(vh * 0.9)));

    if let Some(saved) = settings.window {
        let rect = Bounds::new(
            point(px(saved.x), px(saved.y)),
            size(px(saved.width.max(min.width.as_f32())), px(saved.height.max(min.height.as_f32()))),
        );
        let center = rect.center();
        let on_screen = cx.displays().iter().any(|d| d.visible_bounds().contains(&center));
        if on_screen {
            return (
                if saved.maximized { WindowBounds::Maximized(rect) } else { WindowBounds::Windowed(rect) },
                min,
            );
        }
    }

    let w = (vw * 0.72).clamp(min.width.as_f32(), 1320.0).min(vw * 0.95);
    let h = (vh * 0.8).clamp(min.height.as_f32(), 900.0).min(vh * 0.92);
    let origin = point(
        visible.origin.x + px((vw - w) / 2.0),
        visible.origin.y + px((vh - h) / 2.0),
    );
    (WindowBounds::Windowed(Bounds::new(origin, size(px(w), px(h)))), min)
}

/// Reads the window's restorable bounds for saving.
pub fn saved_window(window: &Window) -> SavedWindow {
    let (bounds, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(b) => (b, false),
        WindowBounds::Maximized(b) => (b, true),
        WindowBounds::Fullscreen(b) => (b, false),
    };
    SavedWindow {
        x: bounds.origin.x.as_f32(),
        y: bounds.origin.y.as_f32(),
        width: bounds.size.width.as_f32(),
        height: bounds.size.height.as_f32(),
        maximized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fills_rows() {
        // 900 dp of content with 236 dp cards and 12 dp gaps: 3 columns.
        assert_eq!(grid_columns(900.0, 236.0, 12.0), 3);
        assert_eq!(grid_columns(1468.0, 236.0, 12.0), 5);
        // Narrower than one card: a single full-width column.
        assert_eq!(grid_columns(200.0, 236.0, 12.0), 1);
    }

    #[test]
    fn sfnt_normalizer_unifies_vertical_metrics_and_aliases_family() {
        let mut copy = GOOGLE_SANS_TTF.to_vec();
        assert!(normalize_sfnt_for_ui(&mut copy, "Google Sans", "BoxPilot UI"));
        // Verify GoogleSans.ttf itself already carries the unified 968/-241 metrics.
        let num = u16::from_be_bytes([GOOGLE_SANS_TTF[4], GOOGLE_SANS_TTF[5]]) as usize;
        for i in 0..num {
            let rec = 12 + i * 16;
            let off = u32::from_be_bytes([
                GOOGLE_SANS_TTF[rec + 8],
                GOOGLE_SANS_TTF[rec + 9],
                GOOGLE_SANS_TTF[rec + 10],
                GOOGLE_SANS_TTF[rec + 11],
            ]) as usize;
            if &GOOGLE_SANS_TTF[rec..rec + 4] == b"OS/2" {
                let w_asc = u16::from_be_bytes([GOOGLE_SANS_TTF[off + 74], GOOGLE_SANS_TTF[off + 75]]);
                let w_desc = u16::from_be_bytes([GOOGLE_SANS_TTF[off + 76], GOOGLE_SANS_TTF[off + 77]]);
                assert_eq!((w_asc, w_desc), (968, 241));
            }
        }
    }
}
