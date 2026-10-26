//! Physically based motion. Every transition is a damped harmonic oscillator
//! (mass–spring–damper) driven by GPUI's analytic spring solver, so motion is
//! frame-rate independent, can be interrupted mid-flight and keeps its
//! velocity when retargeted. Windows "reduce motion" settings are honoured by
//! GPUI automatically.

use gpui_kit::AnimationExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, SpringAnimation, SpringConfig, px};

/// Damping ratio ζ ≈ 0.78: quick with a barely visible overshoot.
pub const SNAPPY: SpringConfig = SpringConfig::new(420.0, 32.0, 1.0);
/// ζ ≈ 0.55: a lively settle for things that slide (indicators, thumbs).
pub const BOUNCY: SpringConfig = SpringConfig::new(360.0, 21.0, 1.0);
/// ζ = 1.0: critically damped, no overshoot (opacity, bar heights, numbers).
pub const SMOOTH: SpringConfig = SpringConfig::new(220.0, 29.66, 1.0);
/// ζ ≈ 0.85, heavier mass: soft entrances.
pub const GENTLE: SpringConfig = SpringConfig::new(180.0, 25.0, 1.1);
/// Softer spring used for the trailing edge of the navigation indicator.
pub const TRAILING: SpringConfig = SpringConfig::new(200.0, 24.0, 1.0);

/// Fades and lifts an element into place when it is mounted. `delay` staggers
/// lists without timers: the spring simply starts further away (`from < 0`),
/// so later items arrive later while obeying the same physics.
pub fn enter(el: Div, id: impl Into<ElementId>, delay: f32) -> impl IntoElement {
    el.relative().with_spring(
        id,
        SpringAnimation::new(GENTLE).to(1.0f32).from(-delay.max(0.0)),
        |el, p| {
            let shown = p.clamp(0.0, 1.0);
            el.opacity(shown).top(px((1.0 - shown) * 14.0))
        },
    )
}

/// Stagger offset for the `i`-th list item.
pub fn stagger(i: usize) -> f32 {
    (i.min(10) as f32) * 0.12
}
