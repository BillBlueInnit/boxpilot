//! Smooth scrolling for any GPUI scroll container.
//!
//! GPUI applies wheel deltas instantly. This wrapper watches the container's
//! [`ScrollHandle`]: whenever the offset jumps (wheel notch, keyboard, scrollbar,
//! programmatic scroll), the jump becomes the new *target* and the visible
//! offset follows it on a critically damped spring. Consecutive notches add to
//! the target, so a fast spin travels the full distance, and a precise
//! touchpad (already smooth) just gets a touch of inertia.

use std::time::Instant;

use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Point, ScrollHandle, SpringConfig, SpringState, Window, point, px,
};

/// ζ = 1.0 (critically damped): fast, no overshoot past the scroll target.
const SCROLL_SPRING: SpringConfig = SpringConfig::new(520.0, 45.6, 1.0);
/// Distance (px) under which the spring is considered settled.
const EPSILON: f32 = 0.35;

pub struct SmoothScroll {
    id: ElementId,
    handle: ScrollHandle,
    child: Option<AnyElement>,
}

/// Wraps `child`, a scroll container tracking `handle`, with spring smoothing.
pub fn smooth_scroll(id: impl Into<ElementId>, handle: &ScrollHandle, child: impl IntoElement) -> SmoothScroll {
    SmoothScroll {
        id: id.into(),
        handle: handle.clone(),
        child: Some(child.into_any_element()),
    }
}

#[derive(Clone, Copy)]
struct Axis {
    spring: SpringState,
    target: f32,
}

struct State {
    x: Axis,
    y: Axis,
    /// The offset this element last wrote; anything else is a new jump.
    written: Point<Pixels>,
    updated: Instant,
}

impl IntoElement for SmoothScroll {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

fn step(axis: &mut Axis, jump: f32, min: f32, dt: f32) -> bool {
    if jump != 0.0 {
        axis.target = (axis.target + jump).clamp(min, 0.0);
    } else {
        axis.target = axis.target.clamp(min, 0.0);
    }
    axis.spring = SCROLL_SPRING.step(axis.spring, axis.target, dt);
    let settled = (axis.spring.position - axis.target).abs() < EPSILON && axis.spring.velocity.abs() < 5.0;
    if settled {
        axis.spring = SpringState { position: axis.target, velocity: 0.0 };
    }
    settled
}

impl Element for SmoothScroll {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self.child.take().expect("request_layout runs once");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let handle = self.handle.clone();
        let reduce_motion = cx.reduce_motion();
        window.with_element_state(id.unwrap(), |state: Option<State>, window| {
            let now = Instant::now();
            let actual = handle.offset();
            let max = handle.max_offset();
            let mut state = state.unwrap_or_else(|| State {
                x: Axis { spring: SpringState { position: actual.x.as_f32(), velocity: 0.0 }, target: actual.x.as_f32() },
                y: Axis { spring: SpringState { position: actual.y.as_f32(), velocity: 0.0 }, target: actual.y.as_f32() },
                written: actual,
                updated: now,
            });
            let jump = actual - state.written;
            let dt = now.duration_since(state.updated).as_secs_f32().min(1.0 / 20.0);
            let settled_x = step(&mut state.x, jump.x.as_f32(), -max.x.as_f32(), dt);
            let settled_y = step(&mut state.y, jump.y.as_f32(), -max.y.as_f32(), dt);
            if reduce_motion {
                state.x.spring = SpringState { position: state.x.target, velocity: 0.0 };
                state.y.spring = SpringState { position: state.y.target, velocity: 0.0 };
            } else if !(settled_x && settled_y) {
                window.request_animation_frame();
            }
            let visual = point(px(state.x.spring.position), px(state.y.spring.position));
            handle.set_offset(visual);
            state.written = visual;
            state.updated = now;
            ((), state)
        });
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simulate(jumps: &[(usize, f32)], frames: usize) -> Vec<f32> {
        let mut axis = Axis { spring: SpringState::default(), target: 0.0 };
        (0..frames)
            .map(|f| {
                let jump = jumps.iter().filter(|(at, _)| *at == f).map(|(_, d)| d).sum();
                step(&mut axis, jump, -10_000.0, 1.0 / 60.0);
                axis.spring.position
            })
            .collect()
    }

    #[test]
    fn notches_accumulate_without_overshoot() {
        // Two wheel notches 2 frames apart: travel the full 240 px.
        let path = simulate(&[(0, -120.0), (2, -120.0)], 40);
        assert!(path.windows(2).all(|w| w[1] <= w[0] + 1e-3), "monotonic: {path:?}");
        assert!(path.iter().all(|p| *p >= -240.0 - 0.01), "no overshoot");
        // Visibly animated: the first frame covers only part of the distance...
        assert!(path[0] > -120.0 && path[0] < -5.0, "first frame {}", path[0]);
        // ...is nearly there after a quarter second and settled by half a second.
        assert!((path[15] + 240.0).abs() < 10.0, "at 250 ms: {}", path[15]);
        assert_eq!(path[30], -240.0);
    }

    #[test]
    fn clamps_to_content_bounds() {
        let mut axis = Axis { spring: SpringState::default(), target: 0.0 };
        for _ in 0..30 {
            step(&mut axis, 50.0, -300.0, 1.0 / 60.0);
        }
        assert_eq!(axis.target, 0.0);
        assert!(axis.spring.position <= 0.0);
    }
}
