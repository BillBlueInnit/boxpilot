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

use std::sync::atomic::{AtomicU32, Ordering};

use boxpilot_core::model::{SavedWindow, Settings};
use gpui_kit::component::Theme;
use gpui_kit::{App, Bounds, Pixels, Size, Window, WindowBounds, point, px, size};

const BASE_FONT: f32 = 16.0;

/// Current interface scale as f32 bits (1.0 = 100 %).
static SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000);
/// The OS text-size factor, read at start-up and when the window regains focus.
static SYSTEM_SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000);

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

/// Applies the interface scale; returns whether it changed.
pub fn apply_scale(settings: &Settings, cx: &mut App) -> bool {
    let s = wanted_scale(settings);
    let changed = (s - scale()).abs() > f32::EPSILON;
    SCALE.store(s.to_bits(), Ordering::Relaxed);
    // Component controls are sized in rems; the root derives rem from this.
    let theme = Theme::global_mut(cx);
    theme.font_size = px(BASE_FONT * s);
    theme.mono_font_size = px(13.0 * s);
    changed
}

/// Layout decisions for the current window width.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Sidebar collapsed to icons.
    pub compact_nav: bool,
    /// Tight spacing for small windows.
    pub narrow: bool,
    pub sidebar: f32,
    /// Horizontal page padding.
    pub pad: f32,
    /// Width available to page content, in `dp`.
    pub content: f32,
}

pub fn metrics(window: &Window) -> Metrics {
    let width = window.viewport_size().width.as_f32() / scale();
    let compact_nav = width < 1000.0;
    let narrow = width < 820.0;
    let sidebar = if compact_nav { 64.0 } else { 216.0 };
    let pad = if narrow { 16.0 } else { 24.0 };
    let content = (width - sidebar - pad * 2.0).clamp(240.0, 1180.0);
    Metrics { compact_nav, narrow, sidebar, pad, content }
}

/// Splits `content` into equal columns of at least `min` (gap included),
/// returning the column width.
pub fn grid_column(content: f32, min: f32, gap: f32) -> f32 {
    let cols = ((content + gap) / (min + gap)).floor().max(1.0);
    (content - gap * (cols - 1.0)) / cols
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
        let w = grid_column(900.0, 236.0, 12.0);
        assert!((w * 3.0 + 24.0 - 900.0).abs() < 0.01 && w >= 236.0);
        // Narrower than one card: a single full-width column.
        assert_eq!(grid_column(200.0, 236.0, 12.0), 200.0);
    }
}
