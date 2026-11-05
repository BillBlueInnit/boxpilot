//! Borderless window chrome that defers to each platform's window manager
//! and draws the window controls in that platform's own style.
//!
//! * Windows: caption buttons at the native 46×32 size with WinUI colours and
//!   glyphs drawn on the device-pixel grid (the ✕ from Segoe Fluent Icons /
//!   Segoe MDL2 Assets), and are
//!   reported to the OS as `HTMINBUTTON` / `HTMAXBUTTON` / `HTCLOSE` / `HTCAPTION`,
//!   so Windows handles clicks, double-click to maximize, Aero Snap, the system
//!   menu and the Windows 11 snap-layouts flyout itself.
//! * macOS: AppKit keeps its real traffic lights and title-bar dragging.
//! * Linux: Adwaita (GNOME) or Breeze (KDE) style buttons, ordered and placed
//!   per the desktop's `button-layout`; with server-side decorations the window
//!   manager draws its own.

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Hsla, Stateful, Window, div, px};

/// Height of the caption area above the page content.
pub const CAPTION_H: f32 = if cfg!(windows) { 32.0 } else { 38.0 };
/// Room for the macOS traffic lights above the sidebar logo.
pub const SIDEBAR_TOP_INSET: f32 = if cfg!(target_os = "macos") { 26.0 } else { 0.0 };

/// An empty area that moves the window when dragged. It must not contain
/// interactive elements: on Windows the whole area is reported as caption.
pub fn drag_region(id: impl Into<ElementId>) -> Stateful<Div> {
    let d = div().id(id);
    #[cfg(windows)]
    let d = d.window_control_area(gpui_kit::WindowControlArea::Drag);
    #[cfg(target_os = "linux")]
    let d = d
        .on_mouse_down(gpui_kit::MouseButton::Left, |e, window, _| {
            if e.click_count >= 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
        .on_mouse_down(gpui_kit::MouseButton::Right, |e, window, _| window.show_window_menu(e.position));
    d
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Right,
}

/// The caption row: a drag area plus right-side window controls.
pub fn caption_bar(window: &Window, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .h(px(CAPTION_H))
        .flex_none()
        .child(drag_region("drag-caption").flex_1().h_full())
        .children(window_controls(Side::Right, window, cx))
        .into_any_element()
}

/// Window controls for one side of the window, if the platform puts any there.
pub fn window_controls(side: Side, window: &Window, cx: &App) -> Option<AnyElement> {
    if cfg!(target_os = "macos") || !client_decorated(window) {
        return None;
    }
    let supported = window.window_controls();
    let controls: Vec<Control> = layout(side)
        .into_iter()
        .filter(|c| match c {
            Control::Minimize => supported.minimize,
            Control::Maximize => supported.maximize,
            Control::Close => true,
        })
        .collect();
    if controls.is_empty() {
        return None;
    }
    #[cfg(windows)]
    let row = windows::controls(&controls, window, cx);
    #[cfg(not(windows))]
    let row = linux::controls(&controls, side, window, cx);
    Some(row)
}

#[cfg(windows)]
fn layout(side: Side) -> Vec<Control> {
    match side {
        Side::Right => vec![Control::Minimize, Control::Maximize, Control::Close],
        Side::Left => vec![],
    }
}

#[cfg(not(windows))]
fn layout(side: Side) -> Vec<Control> {
    linux::button_layout(side)
}

/// Linux: whether the compositor frames windows itself. Tiling compositors
/// (niri, sway, Hyprland, river, i3) draw their own borders, shadows and
/// corners around every window, and only tell server-decorated ones that
/// they are tiled. `BOXPILOT_DECORATIONS=client` or `server` overrides it.
pub fn compositor_frames_windows() -> bool {
    static FRAMES: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        match std::env::var("BOXPILOT_DECORATIONS").as_deref() {
            Ok("server") => return true,
            Ok("client") => return false,
            _ => {}
        }
        ["NIRI_SOCKET", "SWAYSOCK", "HYPRLAND_INSTANCE_SIGNATURE", "I3SOCK"]
            .iter()
            .any(|v| std::env::var_os(v).is_some())
            || std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| {
                d.split(':')
                    .any(|d| matches!(d.to_ascii_lowercase().as_str(), "niri" | "sway" | "hyprland" | "river" | "i3"))
            })
    });
    cfg!(target_os = "linux") && *FRAMES
}

/// The decorations to ask for on Linux: the compositor's where it frames
/// windows anyway (sway honours that; niri and Hyprland keep the window
/// client-decorated, framed by gpui-kit's Root).
pub fn linux_decorations() -> Option<gpui_kit::WindowDecorations> {
    use gpui_kit::WindowDecorations::{Client, Server};
    cfg!(target_os = "linux").then(|| if compositor_frames_windows() { Server } else { Client })
}

/// Whether this window draws its own decorations (always on Windows and
/// macOS with a hidden title bar; on Linux it depends on the compositor).
fn client_decorated(window: &Window) -> bool {
    #[cfg(target_os = "linux")]
    {
        matches!(window.window_decorations(), gpui_kit::Decorations::Client { .. })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        true
    }
}

/// Paints a minimize, maximize or restore glyph `size` px wide, centred in
/// its box, on whole device pixels. Font and SVG glyphs land on fractional
/// pixels at 125 % / 150 % scale and draw each 1 px stroke as two half-tone
/// lines (a doubled square); Windows hints its own glyphs onto the pixel grid.
/// Takes `hover` while the pointer is within `area` around the glyph's centre.
fn grid_glyph(control: Control, maximized: bool, size: f32, color: Hsla, hover: Hsla, area: (f32, f32)) -> impl IntoElement {
    use gpui_kit::{Bounds, canvas, fill, point};
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let s = window.scale_factor();
            let on_grid = |v: f32| (v * s).round() / s;
            let line = s.floor().max(1.0) / s;
            let side = on_grid(size);
            let (cx, cy) = (
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
                f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
            );
            let mouse = window.mouse_position();
            let (mx, my) = (f32::from(mouse.x), f32::from(mouse.y));
            let over = (mx - cx).abs() <= area.0 / 2.0 && (my - cy).abs() <= area.1 / 2.0;
            let color = if over { hover } else { color };
            let (x0, y0) = (on_grid(cx - side / 2.0), on_grid(cy - side / 2.0));
            let mut rect = |x: f32, y: f32, w: f32, h: f32| {
                window.paint_quad(fill(Bounds::new(point(px(x), px(y)), gpui_kit::size(px(w), px(h))), color));
            };
            let mut outline = |x: f32, y: f32, w: f32, h: f32| {
                rect(x, y, w, line);
                rect(x, y + h - line, w, line);
                rect(x, y, line, h);
                rect(x + w - line, y, line, h);
            };
            match control {
                Control::Minimize => rect(x0, on_grid(cy), side, line),
                Control::Maximize if maximized => {
                    // Two overlapping windows: the back one shows its top and right edges.
                    let inner = on_grid(size * 0.8);
                    let off = side - inner;
                    outline(x0, y0 + off, inner, inner);
                    let mut rect = |x: f32, y: f32, w: f32, h: f32| {
                        window.paint_quad(fill(Bounds::new(point(px(x), px(y)), gpui_kit::size(px(w), px(h))), color));
                    };
                    rect(x0 + off, y0, inner, line);
                    rect(x0 + side - line, y0, line, inner);
                }
                Control::Maximize => outline(x0, y0, side, side),
                Control::Close => {}
            }
        },
    )
    .size_full()
}

#[cfg(windows)]
mod windows {
    use super::*;
    use gpui_kit::{Hsla, WindowControlArea, hsla, rgb};

    /// Windows 11 ships Segoe Fluent Icons; Windows 10 has the same glyphs in
    /// Segoe MDL2 Assets.
    fn icon_font() -> &'static str {
        if boxpilot_core::sys::windows_build() >= 22000 {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        }
    }

    pub fn controls(controls: &[Control], window: &Window, cx: &App) -> AnyElement {
        let maximized = window.is_maximized();
        let active = window.is_window_active();
        h_flex()
            .h_full()
            .flex_none()
            .children(controls.iter().map(|c| button(*c, maximized, active, cx)))
            .into_any_element()
    }

    fn button(control: Control, maximized: bool, active: bool, cx: &App) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        // WinUI caption button brushes.
        let fg: Hsla = if dark { hsla(0., 0., 1., 1.) } else { hsla(0., 0., 0., 0.8956) };
        let fg = if active { fg } else { fg.opacity(0.36) };
        let (hover_bg, pressed_bg, hover_fg): (Hsla, Hsla, Hsla) = match control {
            Control::Close => (rgb(0xC42B1C).into(), rgb(0xC42B1C).into(), rgb(0xFFFFFF).into()),
            _ if dark => (hsla(0., 0., 1., 0.0605), hsla(0., 0., 1., 0.0419), fg.opacity(1.0)),
            _ => (hsla(0., 0., 0., 0.0373), hsla(0., 0., 0., 0.0241), fg.opacity(1.0)),
        };
        let pressed_fg = if control == Control::Close { hsla(0., 0., 1., 0.7) } else { fg.opacity(0.6) };
        div()
            .id(match control {
                Control::Minimize => "caption-min",
                Control::Maximize => "caption-max",
                Control::Close => "caption-close",
            })
            .w(px(46.))
            .h_full()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .font_family(icon_font())
            .relative()
            .text_size(px(10.))
            .text_color(fg)
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .active(move |s| s.bg(pressed_bg).text_color(pressed_fg))
            // Reported to Windows as caption buttons, which gives the native
            // snap-layouts flyout on Maximize. The press itself is handled here
            // (and kept from the native handler) so it also works when the
            // cursor jumps onto the button, e.g. with touch or pen.
            .window_control_area(match control {
                Control::Minimize => WindowControlArea::Min,
                Control::Maximize => WindowControlArea::Max,
                Control::Close => WindowControlArea::Close,
            })
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| match control {
                Control::Minimize => window.minimize_window(),
                Control::Maximize => crate::platform::toggle_maximize(window),
                Control::Close => {
                    if crate::ui::handle_close_request(window, cx) {
                        window.remove_window();
                    }
                }
            })
            // The diagonal ✕ reads fine from the font; the others are drawn
            // on the pixel grid (see `grid_glyph`).
            .map(|d| match control {
                Control::Close => d.child("\u{E8BB}"),
                _ => d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(super::grid_glyph(control, maximized, 10., fg, hover_fg, (46., CAPTION_H))),
                ),
            })
    }
}

#[cfg(not(windows))]
mod linux {
    use super::*;
    use gpui_kit::component::{Icon, IconName};
    use std::sync::LazyLock;

    #[derive(Clone, Copy, PartialEq)]
    enum Style {
        Adwaita,
        Breeze,
    }

    fn style() -> Style {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_uppercase();
        if desktop.contains("KDE") { Style::Breeze } else { Style::Adwaita }
    }

    /// GNOME's `button-layout`, e.g. `appmenu:minimize,maximize,close` or
    /// `close,minimize,maximize:` for left-hand controls.
    static LAYOUT: LazyLock<(Vec<Control>, Vec<Control>)> = LazyLock::new(|| {
        let raw = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
            .unwrap_or_else(|| "appmenu:minimize,maximize,close".into());
        let parse = |part: &str| {
            part.split(',')
                .filter_map(|t| match t.trim() {
                    "minimize" => Some(Control::Minimize),
                    "maximize" => Some(Control::Maximize),
                    "close" => Some(Control::Close),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let (left, right) = raw.split_once(':').unwrap_or(("", raw.as_str()));
        let (left, right) = (parse(left), parse(right));
        if left.is_empty() && right.is_empty() {
            (vec![], vec![Control::Minimize, Control::Maximize, Control::Close])
        } else {
            (left, right)
        }
    });

    pub fn button_layout(side: Side) -> Vec<Control> {
        match side {
            Side::Left => LAYOUT.0.clone(),
            Side::Right => LAYOUT.1.clone(),
        }
    }

    pub fn controls(controls: &[Control], side: Side, window: &Window, cx: &App) -> AnyElement {
        let maximized = window.is_maximized();
        let style = style();
        h_flex()
            .h_full()
            .flex_none()
            .items_center()
            .gap(px(if style == Style::Adwaita { 12. } else { 6. }))
            .when(side == Side::Right, |d| d.pr(px(10.)))
            .when(side == Side::Left, |d| d.pl(px(10.)))
            .children(controls.iter().map(|c| button(*c, maximized, style, cx)))
            .into_any_element()
    }

    fn button(control: Control, maximized: bool, style: Style, cx: &App) -> impl IntoElement {
        let t = cx.theme();
        let fg = t.foreground;
        // Adwaita: always-visible round buttons. Breeze: the circle appears on
        // hover, red for close.
        let (rest, hover, pressed, hover_fg) = match (style, control) {
            (Style::Adwaita, _) => (fg.opacity(0.1), fg.opacity(0.15), fg.opacity(0.3), fg),
            (Style::Breeze, Control::Close) => (fg.opacity(0.0), t.danger, t.danger.opacity(0.8), t.danger_foreground),
            (Style::Breeze, _) => (fg.opacity(0.0), fg.opacity(0.12), fg.opacity(0.2), fg),
        };
        div()
            .id(match control {
                Control::Minimize => "caption-min",
                Control::Maximize => "caption-max",
                Control::Close => "caption-close",
            })
            .size(px(24.))
            .relative()
            .rounded_full()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .bg(rest)
            .text_color(fg)
            .hover(move |s| s.bg(hover).text_color(hover_fg))
            .active(move |s| s.bg(pressed))
            // Keep the drag handler of the caption from grabbing the click.
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                match control {
                    Control::Minimize => window.minimize_window(),
                    Control::Maximize => crate::platform::toggle_maximize(window),
                    Control::Close => {
                        if crate::ui::handle_close_request(window, cx) {
                            window.remove_window();
                        }
                    }
                }
            })
            // The ✕ is diagonal and reads fine as an icon; the others are
            // drawn on the pixel grid (see `grid_glyph`).
            .map(|d| match control {
                Control::Close => d.child(Icon::new(IconName::WindowClose).size(px(14.))),
                _ => d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(super::grid_glyph(control, maximized, 8., fg, hover_fg, (24., 24.))),
                ),
            })
    }
}
