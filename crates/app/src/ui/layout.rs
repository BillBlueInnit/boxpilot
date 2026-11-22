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

/// Applies the user's chosen UI typeface and monospace font; returns whether the
/// active UI font family changed (for live refresh without restart).
pub fn apply_fonts(settings: &Settings, cx: &mut App) -> bool {
    #[cfg(windows)]
    let system_ui_candidates: &[&str] = &["Segoe UI Variable", "Segoe UI Variable Text", "Segoe UI"];
    #[cfg(not(windows))]
    let system_ui_candidates: &[&str] = &[];

    #[cfg(windows)]
    let mono_candidates: &[&str] = &["Cascadia Mono", "Cascadia Code", "Consolas"];
    #[cfg(target_os = "macos")]
    let mono_candidates: &[&str] = &["SF Mono", "Menlo"];
    #[cfg(all(unix, not(target_os = "macos")))]
    let mono_candidates: &[&str] = &["JetBrains Mono", "Noto Sans Mono", "DejaVu Sans Mono"];

    let system_ui = || -> SharedString {
        pick_installed(system_ui_candidates, cx)
            .map(SharedString::from)
            .or_else(|| DEFAULT_UI_FONT.get().cloned())
            .unwrap_or_else(|| ".SystemUIFont".into())
    };

    let wanted_ui: SharedString = match settings.ui_font {
        UiFont::System => system_ui(),
        UiFont::GoogleSans => {
            if GOOGLE_SANS_LOADED.load(Ordering::Relaxed) {
                "Google Sans".into()
            } else {
                system_ui()
            }
        }
        UiFont::SourceHanSans => pick_source_han_sans(settings.language, cx)
            .map(SharedString::from)
            .unwrap_or_else(system_ui),
    };

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
/// drop back to jagged legacy bitmap/GDI fonts when a smoother CJK font exists.
pub fn font_fallbacks(lang: Language, cx: &App) -> Option<FontFallbacks> {
    #[cfg(windows)]
    {
        let resolved = match lang {
            Language::System => crate::i18n::active_language(),
            other => other,
        };
        let list: &[&str] = match resolved {
            Language::SimplifiedChinese => &[
                "Source Han Sans SC",
                "Source Han Sans CN",
                "思源黑体 CN",
                "Noto Sans SC",
                "Noto Sans CJK SC",
                "HarmonyOS Sans SC",
                "MiSans",
                "Microsoft YaHei UI",
                "Segoe UI",
                "Segoe UI Symbol",
                "Segoe UI Emoji",
            ],
            Language::TraditionalChinese => &[
                "Source Han Sans TC",
                "Source Han Sans TW",
                "思源黑體 TW",
                "Noto Sans TC",
                "Noto Sans CJK TC",
                "Microsoft JhengHei UI",
                "Microsoft YaHei UI",
                "Segoe UI",
                "Segoe UI Symbol",
                "Segoe UI Emoji",
            ],
            _ => &[
                "Source Han Sans SC",
                "Source Han Sans CN",
                "Noto Sans SC",
                "Noto Sans CJK SC",
                "Microsoft YaHei UI",
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
}
