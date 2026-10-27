//! The theme colour: one accent for everything that shows state — primary
//! buttons, switches, selections, focus rings, links, progress — plus the
//! control colours that change on a translucent window.

use boxpilot_core::model::{Accent, Settings};
use gpui_kit::component::Theme;
use gpui_kit::{App, Hsla, rgb};

/// (dark mode, light mode). Dark themes take the light shade of an accent and
/// light themes the deep one, as Windows 11, macOS and GNOME all do, so text
/// on it stays readable in either mode and over any backdrop.
fn shades(a: Accent) -> (u32, u32) {
    match a {
        Accent::Blue | Accent::System => (0x60CDFF, 0x005FB8),
        Accent::Teal => (0x4FD8CF, 0x00777B),
        Accent::Green => (0x6CCB5F, 0x0F7B0F),
        Accent::Violet => (0xB9A3FF, 0x6B48C8),
        Accent::Pink => (0xFF8FC8, 0xBF1F6A),
        Accent::Orange => (0xFFB066, 0xB94A00),
        Accent::Graphite => (0xEDEDED, 0x1F1F1F),
    }
}

/// The accent for the current mode. System reads the OS accent; Blue when
/// there is none to read.
pub fn color(a: Accent, dark: bool) -> Hsla {
    if a == Accent::System {
        if let Some(c) = boxpilot_core::sys::accent_color(dark) {
            return rgb(c).into();
        }
    }
    let (d, l) = shades(a);
    rgb(if dark { d } else { l }).into()
}

/// Black or white text, whichever reads better on `c`.
pub fn text_on(c: Hsla) -> Hsla {
    let c = c.to_rgb();
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let luminance = 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
    if luminance > 0.4 { gpui_kit::hsla(0., 0., 0., 0.9) } else { gpui_kit::white() }
}

/// Writes the accent and the translucent control colours into the global
/// theme, on both the colours and the component tokens (some controls read
/// the tokens). Runs after every theme change, since that resets them.
pub fn apply(settings: &Settings, cx: &mut App) {
    let translucent = super::layout::translucent();
    let theme = Theme::global_mut(cx);
    let dark = theme.is_dark();
    let c = color(settings.accent, dark);
    let fg = text_on(c);
    macro_rules! set {
        ($($field:ident = $value:expr),+ $(,)?) => {$(
            let v: Hsla = $value;
            theme.colors.$field = v;
            theme.tokens.$field = v.into();
        )+};
    }
    set!(
        primary = c,
        primary_hover = c.opacity(0.9),
        primary_active = c.opacity(0.8),
        primary_foreground = fg,
        button_primary = c,
        button_primary_hover = c.opacity(0.9),
        button_primary_active = c.opacity(0.8),
        button_primary_foreground = fg,
        ring = c,
        link = c,
        link_hover = c.opacity(0.85),
        link_active = c.opacity(0.75),
        progress_bar = c,
        slider_bar = c,
        sidebar_primary = c,
        sidebar_primary_foreground = fg,
        selection = c.opacity(0.3),
        caret = c,
        chart_1 = c,
    );
    if translucent {
        // Text boxes take fill and border from `input`: Fluent's control
        // stroke, a faint line instead of a grey frame around a dark slab.
        let stroke = if dark { gpui_kit::white().opacity(0.08) } else { gpui_kit::black().opacity(0.1) };
        set!(input = stroke);
    }
}
