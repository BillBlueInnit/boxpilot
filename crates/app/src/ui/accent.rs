//! BoxPilot's one design language, written into the theme.
//!
//! Every mode and material uses the same layers, after Windows 11's Fluent
//! (whose neutrals macOS and GNOME are close to): a base under the window and
//! sidebar, a raised content layer, cards with a faint veil, controls with a
//! subtle fill and a hairline stroke, hairline dividers, and one theme colour
//! for every selected or active state. A translucent window only swaps the
//! solid base for the system backdrop; nothing else changes style.

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

/// Writes the palette and the theme colour into the global theme, on both the
/// colours and the component tokens (some controls read the tokens). Runs
/// after every theme change, since that resets them.
pub fn apply(settings: &Settings, cx: &mut App) {
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

    // Neutrals (Fluent): text, strokes, fills. Translucent fills and strokes
    // read the same over a solid base and over a backdrop.
    let (white, black) = (gpui_kit::white(), gpui_kit::black());
    let base: Hsla = rgb(if dark { 0x202020 } else { 0xF3F3F3 }).into();
    let text: Hsla = if dark { white } else { rgb(0x141414).into() };
    let text_secondary = if dark { white.opacity(0.76) } else { black.opacity(0.66) };
    let divider = if dark { white.opacity(0.084) } else { black.opacity(0.08) };
    let control_stroke = if dark { white.opacity(0.09) } else { black.opacity(0.11) };
    // Light fills are solid: translucent whites stacked on a translucent card
    // composite grey in GPUI. Dark ones are faint lifts, where that is unseen.
    let (fill, hover, pressed): (Hsla, Hsla, Hsla) = if dark {
        (white.opacity(0.068), white.opacity(0.095), white.opacity(0.04))
    } else {
        (rgb(0xFFFFFF).into(), rgb(0xF6F7F9).into(), rgb(0xEDEFF2).into())
    };
    // Keep controls crisp without muddy shadow smudges or outer 3px glow halos
    // under thin rounded borders (focused inputs/buttons still get a crisp
    // accent border via `theme.ring`).
    theme.shadow = false;
    theme.focus_ring = false;
    let subtle = if dark { white.opacity(0.068) } else { black.opacity(0.05) };
    // Keep dropdown menus and popovers 100% opaque across both translucent and
    // solid window modes so GPUI's DirectX premultiplied blend never bleeds
    // shadows or rings through/around floating surfaces.
    let flyout: Hsla = rgb(if dark { 0x26282E } else { 0xFCFCFD }).into();
    set!(
        background = base,
        foreground = text,
        muted_foreground = text_secondary,
        border = divider,
        sidebar = base,
        sidebar_foreground = text,
        sidebar_border = divider,
        sidebar_accent = subtle,
        sidebar_accent_foreground = text,
        popover = flyout,
        popover_foreground = text,
        // Standard buttons and text boxes share the control fill and stroke.
        input = control_stroke,
        button = fill,
        button_hover = hover,
        button_active = pressed,
        button_foreground = text,
        secondary = fill,
        secondary_hover = hover,
        secondary_active = pressed,
        secondary_foreground = text,
        // Hover rows, badges, tracks, menu highlights: the subtle fill.
        muted = subtle,
        accent = subtle,
        accent_foreground = text,
        list_hover = subtle,
        list_active = subtle,
    );
}
