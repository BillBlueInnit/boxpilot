//! Borderless window chrome that defers to each platform's window manager
//! and draws the window controls in that platform's own style.
//!
//! * Windows: caption buttons use the system icon font (Segoe Fluent Icons /
//!   Segoe MDL2 Assets) at the native 46×32 size with WinUI colours, and are
//!   reported to the OS as `HTMINBUTTON` / `HTMAXBUTTON` / `HTCLOSE` / `HTCAPTION`,
//!   so Windows handles clicks, double-click to maximize, Aero Snap, the system
//!   menu and the Windows 11 snap-layouts flyout itself.
//! * macOS: AppKit keeps its real traffic lights and title-bar dragging.
//! * Linux: Adwaita (GNOME) or Breeze (KDE) style buttons, ordered and placed
//!   per the desktop's `button-layout`; with server-side decorations the window
//!   manager draws its own.

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Stateful, Window, div, px};

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
/// they are tiled; BoxPilot's frame there would add a second border and
/// shadow inside a transparent margin they paint behind.
/// `BOXPILOT_DECORATIONS=client` (BoxPilot's frame) or `server` overrides it.
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
/// client-decorated, and BoxPilot then draws no frame of its own).
pub fn linux_decorations() -> Option<gpui_kit::WindowDecorations> {
    use gpui_kit::WindowDecorations::{Client, Server};
    cfg!(target_os = "linux").then(|| if compositor_frames_windows() { Server } else { Client })
}

/// The window where the compositor frames it (`compositor_frames_windows`):
/// the content edge to edge, with no border or shadow of BoxPilot's (a
/// second, soft line inside the compositor's at fractional scales), only
/// the resize edges a client-decorated window must offer itself.
pub fn bare_frame(content: impl IntoElement, window: &mut Window) -> Stateful<Div> {
    use gpui_kit::{CursorStyle, MouseButton, ResizeEdge};
    const EDGE: f32 = 6.0;
    window.set_client_inset(px(0.));
    let resizable = window.is_resizable()
        && matches!(window.window_decorations(), gpui_kit::Decorations::Client { tiling } if !tiling.is_tiled());
    let zone = |edge: ResizeEdge, cursor: CursorStyle| {
        div()
            .absolute()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.start_window_resize(edge);
            })
    };
    let (band, corner) = (px(EDGE), px(EDGE * 2.));
    div().id("bare-frame").relative().size_full().child(content).when(resizable, |d| {
        d.child(zone(ResizeEdge::Top, CursorStyle::ResizeUpDown).top_0().left_0().right_0().h(band))
            .child(zone(ResizeEdge::Bottom, CursorStyle::ResizeUpDown).bottom_0().left_0().right_0().h(band))
            .child(zone(ResizeEdge::Left, CursorStyle::ResizeLeftRight).top_0().bottom_0().left_0().w(band))
            .child(zone(ResizeEdge::Right, CursorStyle::ResizeLeftRight).top_0().bottom_0().right_0().w(band))
            .child(zone(ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight).top_0().left_0().size(corner))
            .child(zone(ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft).top_0().right_0().size(corner))
            .child(zone(ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft).bottom_0().left_0().size(corner))
            .child(zone(ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight).bottom_0().right_0().size(corner))
    })
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
        let glyph = match control {
            Control::Minimize => "\u{E921}",
            Control::Maximize if maximized => "\u{E923}",
            Control::Maximize => "\u{E922}",
            Control::Close => "\u{E8BB}",
        };
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
            .child(glyph)
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
        let icon = match control {
            Control::Minimize => IconName::WindowMinimize,
            Control::Maximize if maximized => IconName::WindowRestore,
            Control::Maximize => IconName::WindowMaximize,
            Control::Close => IconName::WindowClose,
        };
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
            .child(Icon::new(icon).size(px(14.)))
    }
}
