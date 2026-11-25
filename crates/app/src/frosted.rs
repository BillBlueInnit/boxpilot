//! Windows 11 macOS-style GPU compositor vibrancy engine.
//!
//! GPUI attaches its premultiplied-alpha DXGI swapchain to the window's
//! `topmost = true` DirectComposition slot. This module attaches a pure
//! `Windows.UI.Composition` visual tree to the free `topmost = false` slot
//! directly underneath GPUI:
//!
//! 1. `blur_visual`: live `HostBackdropBrush` → Direct2D `Saturation` → Direct2D
//!    `GaussianBlur` (28 px, `Hard` border mode) — zero Windows Acrylic grain noise,
//!    smooth wide-radius macOS blur, subtle vibrancy color boost.
//! 2. `tint_visual`: clean porcelain white (Light) or deep neutral charcoal (Dark)
//!    luminosity veil composited on the GPU *under* GPUI's swapchain so GPUI never
//!    stacks two translucent light fills (preventing grey card compositing).

use std::cell::RefCell;
use std::sync::Mutex;

use windows::Foundation::Numerics::Vector2;
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl,
};
use windows::System::DispatcherQueueController;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionBrush, CompositionEffectSourceParameter, Compositor, ContainerVisual, SpriteVisual,
};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, HWND};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    GRAPHICS_EFFECT_PROPERTY_MAPPING, IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
};
use windows_core::{GUID, HSTRING, Interface, PCWSTR, Result, implement};

/// macOS-matched Gaussian blur standard deviation (px) and vibrancy saturation.
/// `30 px` matches macOS `NSVisualEffectView` softness — grain-free while
/// letting wallpaper gradients and silhouettes glow clearly at Clear / Ultra.
const BLUR_STD_DEV: f32 = 30.0;
const LIGHT_SATURATION: f32 = 1.60;
const DARK_SATURATION: f32 = 1.60;

/// Per-tier compositor tint alpha `[Solid, Subtle, Balanced, Clear, Ultra]`.
/// Light and Dark share the same perceived transparency curve (Light sits
/// 0.02 lower so that once the subtle white card lift is added on top, Light
/// and Dark feel optically identical at every tier, matching macOS).
const LIGHT_TINT_ALPHA: [f32; 5] = [0.95, 0.70, 0.48, 0.28, 0.12];
const DARK_TINT_ALPHA: [f32; 5] = [0.95, 0.72, 0.50, 0.30, 0.14];

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct SaturationEffect {
    name: Mutex<HSTRING>,
    saturation: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for SaturationEffect_Impl {}

impl IGraphicsEffect_Impl for SaturationEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.lock().unwrap().clone())
    }

    fn SetName(&self, name: &HSTRING) -> Result<()> {
        *self.name.lock().unwrap() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for SaturationEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1Saturation)
    }

    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(1)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        match index {
            0 => PropertyValue::CreateSingle(self.saturation)?.cast(),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        match index {
            0 => Ok(self.source.clone()),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct GaussianBlurEffect {
    name: Mutex<HSTRING>,
    std_dev: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for GaussianBlurEffect_Impl {}

impl IGraphicsEffect_Impl for GaussianBlurEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.lock().unwrap().clone())
    }

    fn SetName(&self, name: &HSTRING) -> Result<()> {
        *self.name.lock().unwrap() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for GaussianBlurEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }

    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(3)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        match index {
            0 => PropertyValue::CreateSingle(self.std_dev)?.cast(),
            1 => PropertyValue::CreateUInt32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)?.cast(),
            // D2D1_BORDER_MODE_HARD = 1: clamp edges so blur never vignettes at the window borders.
            2 => PropertyValue::CreateUInt32(1)?.cast(),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        match index {
            0 => Ok(self.source.clone()),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

struct FrostedState {
    _queue: Option<DispatcherQueueController>,
    compositor: Compositor,
    target_hwnd: isize,
    target: Option<DesktopWindowTarget>,
    _root: Option<ContainerVisual>,
    blur_visual: Option<SpriteVisual>,
    tint_visual: Option<SpriteVisual>,
}

thread_local! {
    static STATE: RefCell<Option<FrostedState>> = const { RefCell::new(None) };
}

fn ensure_state(slot: &mut Option<FrostedState>) -> Result<&mut FrostedState> {
    if slot.is_none() {
        let options = DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        };
        let queue = unsafe { CreateDispatcherQueueController(options) }.ok();
        let compositor = Compositor::new()?;
        *slot = Some(FrostedState {
            _queue: queue,
            compositor,
            target_hwnd: 0,
            target: None,
            _root: None,
            blur_visual: None,
            tint_visual: None,
        });
    }
    Ok(slot.as_mut().unwrap())
}

fn build_vibrancy_brush(compositor: &Compositor, dark: bool, blur_std_dev: f32) -> Result<CompositionBrush> {
    let source_name = HSTRING::from("backdrop");
    let source_param: IGraphicsEffectSource = CompositionEffectSourceParameter::Create(&source_name)?.cast()?;

    let saturation: IGraphicsEffectSource = SaturationEffect {
        name: Mutex::new(HSTRING::from("Saturation")),
        saturation: if dark { DARK_SATURATION } else { LIGHT_SATURATION },
        source: source_param,
    }
    .into();

    let blur: IGraphicsEffect = GaussianBlurEffect {
        name: Mutex::new(HSTRING::from("Blur")),
        std_dev: blur_std_dev,
        source: saturation,
    }
    .into();

    let factory = compositor.CreateEffectFactory(&blur)?;
    let effect_brush = factory.CreateBrush()?;
    let host_backdrop = compositor.CreateHostBackdropBrush()?;
    effect_brush.SetSourceParameter(&source_name, &host_backdrop)?;
    effect_brush.cast()
}

fn tint_color(dark: bool, tier: usize) -> Color {
    let idx = tier.min(4);
    let alpha = if dark { DARK_TINT_ALPHA[idx] } else { LIGHT_TINT_ALPHA[idx] };
    let a = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    if dark {
        // Neutral macOS smoked charcoal (lets warm/cool desktop light breathe through).
        Color { A: a, R: 0x24, G: 0x25, B: 0x29 }
    } else {
        // Soft neutral porcelain frost matching macOS Light vibrancy.
        Color { A: a, R: 0xF4, G: 0xF5, B: 0xF7 }
    }
}

fn ensure_target(st: &mut FrostedState, raw_hwnd: *mut std::ffi::c_void) -> Result<()> {
    let hwnd_key = raw_hwnd as isize;
    let hwnd = HWND(raw_hwnd);
    if st.target_hwnd != hwnd_key || st.target.is_none() || st.blur_visual.is_none() || st.tint_visual.is_none() {
        if let Some(old) = st.target.take() {
            let _ = old.SetRoot(None);
        }
        let interop: ICompositorDesktopInterop = st.compositor.cast()?;
        // isTopmost = false places our visuals directly behind GPUI's
        // topmost = true swapchain on the exact same HWND.
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false) }?;
        let root = st.compositor.CreateContainerVisual()?;
        root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;

        let blur_visual = st.compositor.CreateSpriteVisual()?;
        blur_visual.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;

        let tint_visual = st.compositor.CreateSpriteVisual()?;
        tint_visual.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;

        let children = root.Children()?;
        children.InsertAtTop(&blur_visual)?;
        children.InsertAtTop(&tint_visual)?;
        target.SetRoot(&root)?;

        st.target_hwnd = hwnd_key;
        st.target = Some(target);
        st._root = Some(root);
        st.blur_visual = Some(blur_visual);
        st.tint_visual = Some(tint_visual);
    }
    Ok(())
}

/// Activates or updates the GPU compositor vibrancy layer underneath `hwnd`.
/// Returns `true` if the compositor layer is active, `false` on unsupported setups.
pub fn apply(raw_hwnd: *mut std::ffi::c_void, dark: bool, tier: usize) -> bool {
    STATE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let res: Result<()> = (|| {
            let st = ensure_state(&mut slot)?;
            ensure_target(st, raw_hwnd)?;
            let blur_brush = build_vibrancy_brush(&st.compositor, dark, BLUR_STD_DEV)?;
            if let Some(blur_visual) = &st.blur_visual {
                blur_visual.SetIsVisible(true)?;
                blur_visual.SetBrush(&blur_brush)?;
            }
            let tint_brush = st.compositor.CreateColorBrushWithColor(tint_color(dark, tier))?;
            if let Some(tint_visual) = &st.tint_visual {
                tint_visual.SetBrush(&tint_brush)?;
            }
            Ok(())
        })();
        match res {
            Ok(()) => true,
            Err(e) => {
                log::warn!("frosted compositor vibrancy unavailable: {e}");
                false
            }
        }
    })
}

/// Removes the compositor vibrancy layer when switching to Mica, Acrylic, or Opaque.
pub fn clear() {
    STATE.with(|cell| {
        if let Some(st) = cell.borrow_mut().as_mut() {
            if let Some(target) = st.target.take() {
                let _ = target.SetRoot(None);
            }
            st._root = None;
            st.blur_visual = None;
            st.tint_visual = None;
            st.target_hwnd = 0;
        }
    });
}
