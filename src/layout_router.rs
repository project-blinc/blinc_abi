//! Binding node properties to constants, signals and computeds.
//!
//! Every router takes `kind`: 0 applies the constant argument, 1 binds the
//! signal handle, 2 binds the computed handle. A binding also applies its
//! current value at once, because Blinc's bindings only write on change.
//! Writes are queued; `blinc_tree_flush` applies them.
//!
//! Enum-valued properties use the integer codes of the Haxe enum abstracts in
//! `ashui.types.Style`. Codes out of range fall back to the property's default.
//!
//! Properties a text node is measured by also change its measure context.
//! Blinc's queue carries only render and layout writes, so those bindings
//! record the change here and `blinc_tree_flush` applies it through
//! `LayoutTree::update_text`.

use crate::hl::{handle_ref, opt_string_from};
use crate::layout_props;
use crate::reactive::{AnyComputed, AnySignal, Slot};
use crate::types::{GlassEffects, Value};
use blinc_core::CornerShape;
use blinc_layout::binding::{
    register_typed, register_typed_computed, register_typed_layout, register_typed_layout_computed,
};
use blinc_layout::div::{FontWeight, TextAlign};
use blinc_layout::element::{BorderSide, RenderProps};
use blinc_layout::element_style::FontStyle;
use blinc_layout::property::PropertyId;
use blinc_layout::stateful::{queue_layout_update_partial, queue_prop_update_partial};
use blinc_layout::tree::{LayoutNodeId, TextMeasureContext};
use hl_abi::{define_prim, vbyte};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use taffy::prelude::*;

const KIND_CONST: i32 = 0;
const KIND_SIGNAL: i32 = 1;
const KIND_COMPUTED: i32 = 2;

/// `PropertyId` in declaration order, which `ashui.layout.PropertyId` mirrors.
const PROPERTIES: [PropertyId; 43] = {
    use PropertyId::*;
    [
        Background,
        BorderColor,
        BorderWidth,
        CornerRadius,
        Opacity,
        Transform,
        Shadow,
        Color,
        Filter,
        AccentColor,
        Width,
        Height,
        MinWidth,
        MaxWidth,
        MinHeight,
        MaxHeight,
        Padding,
        Margin,
        Gap,
        FlexDirection,
        AlignItems,
        JustifyContent,
        AlignSelf,
        FlexGrow,
        FlexShrink,
        FlexWrap,
        FlexBasis,
        Display,
        Overflow,
        Position,
        Top,
        Right,
        Bottom,
        Left,
        FontSize,
        FontFamily,
        FontWeight,
        FontStyle,
        LetterSpacing,
        LineHeight,
        TextAlign,
        TextContent,
        Compound,
    ]
};

fn property(raw: i32) -> Option<PropertyId> {
    usize::try_from(raw)
        .ok()
        .and_then(|i| PROPERTIES.get(i))
        .copied()
}

// ============================================================================
// TEXT MEASURE CONTEXT
// ============================================================================

pub type TextWrite = Box<dyn FnOnce(&mut TextMeasureContext) + Send>;

static PENDING_TEXT: Mutex<Vec<(LayoutNodeId, TextWrite)>> = Mutex::new(Vec::new());

fn record_text(node: LayoutNodeId, write: impl FnOnce(&mut TextMeasureContext) + Send + 'static) {
    PENDING_TEXT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((node, Box::new(write)));
}

/// The identity of each backdrop colour filter: brightness, contrast,
/// grayscale, hue-rotate, invert, saturate, sepia.
pub const BACKDROP_IDENTITY: [f32; 7] = [1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// A backdrop colour filter's change: the node, which filter (an index into `BACKDROP_IDENTITY`), its value.
static PENDING_BACKDROP: Mutex<Vec<(LayoutNodeId, usize, f32)>> = Mutex::new(Vec::new());

fn record_backdrop(node: LayoutNodeId, filter: usize, value: f32) {
    PENDING_BACKDROP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((node, filter, value));
}

/// Backdrop filter changes recorded since the last call, oldest first. Blinc's
/// render props have no backdrop filter, so ashui keeps them beside the tree.
pub fn take_pending_backdrop() -> Vec<(LayoutNodeId, usize, f32)> {
    std::mem::take(&mut *PENDING_BACKDROP.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Kept beside render props because Blinc's glass style has no configurable bevel or dispersion.
static PENDING_GLASS: Mutex<Vec<(LayoutNodeId, Option<GlassEffects>)>> = Mutex::new(Vec::new());

fn record_glass(node: LayoutNodeId, effects: Option<GlassEffects>) {
    PENDING_GLASS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((node, effects));
}

pub fn take_pending_glass() -> Vec<(LayoutNodeId, Option<GlassEffects>)> {
    std::mem::take(&mut *PENDING_GLASS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Text changes recorded since the last call, oldest first.
pub fn take_pending_text() -> Vec<(LayoutNodeId, TextWrite)> {
    std::mem::take(&mut *PENDING_TEXT.lock().unwrap_or_else(|e| e.into_inner()))
}

// ============================================================================
// BINDING
// ============================================================================

type LayoutWrite<T> = Arc<dyn Fn(&mut Style, T) + Send + Sync>;
type RenderWrite<T> = Arc<dyn Fn(&mut RenderProps, T) + Send + Sync>;

/// Where a property's value goes: into the Taffy style (relayout) or into the
/// node's render props.
enum Write<T> {
    Layout(LayoutWrite<T>),
    Render(RenderWrite<T>),
}

fn render<T>(f: impl Fn(&mut RenderProps, T) + Send + Sync + 'static) -> Option<Write<T>> {
    Some(Write::Render(Arc::new(f)))
}

impl<T: Slot> Write<T> {
    fn queue(&self, node: LayoutNodeId, prop: PropertyId, v: T) {
        match self {
            Write::Layout(w) => {
                let w = Arc::clone(w);
                queue_layout_update_partial(node, prop, prop.side_effects(), move |s| w(s, v));
            }
            Write::Render(w) => {
                let w = Arc::clone(w);
                queue_prop_update_partial(node, prop, prop.side_effects(), move |p| w(p, v));
            }
        }
    }
}

/// # Safety
/// `sig` and `comp` must each be null or a handle of their kind.
unsafe fn bind<T: Slot>(
    node: LayoutNodeId,
    prop: PropertyId,
    kind: i32,
    constant: T,
    sig: *mut c_void,
    comp: *mut c_void,
    write: Write<T>,
) {
    match kind {
        KIND_CONST => write.queue(node, prop, constant),
        KIND_SIGNAL => {
            let Some(state) = (unsafe { handle_ref::<AnySignal>(sig) }).and_then(T::state_view)
            else {
                return;
            };
            match &write {
                Write::Layout(w) => {
                    let w = Arc::clone(w);
                    register_typed_layout(
                        state.signal_id(),
                        node,
                        prop,
                        state.clone(),
                        move |s, v| w(s, v),
                    );
                }
                Write::Render(w) => {
                    let w = Arc::clone(w);
                    register_typed(state.signal_id(), node, prop, state.clone(), move |p, v| {
                        w(p, v)
                    });
                }
            }
            write.queue(node, prop, state.try_get().unwrap_or_default());
        }
        KIND_COMPUTED => {
            let Some(c) = (unsafe { handle_ref::<AnyComputed>(comp) }).and_then(T::computed_view)
            else {
                return;
            };
            match &write {
                Write::Layout(w) => {
                    let w = Arc::clone(w);
                    register_typed_layout_computed(
                        c.derived_id(),
                        node,
                        prop,
                        c.clone(),
                        move |s, v| w(s, v),
                    );
                }
                Write::Render(w) => {
                    let w = Arc::clone(w);
                    register_typed_computed(c.derived_id(), node, prop, c.clone(), move |p, v| {
                        w(p, v)
                    });
                }
            }
            write.queue(node, prop, c.try_get().unwrap_or_default());
        }
        _ => {}
    }
}

// ============================================================================
// F32: lengths, flex factors, opacity, typography metrics
// ============================================================================

/// ashui's own number properties, numbered after Blinc's: one side of a
/// box's padding, margin, gap or border, a size as a fraction of the
/// parent's, an outline's width or offset, one side's overflow fade, or a
/// colour filter's amount. Each is updated under the
/// Blinc property it is part of.
const SIDES_BASE: i32 = 43;

fn side_write(node: LayoutNodeId, raw: i32) -> Option<(PropertyId, Write<f32>)> {
    use PropertyId as P;
    Some(match raw - SIDES_BASE {
        // A side's width over the border's.
        17 => (
            P::BorderWidth,
            render(|p, v| side(&mut p.border_sides.top).width = v)?,
        ),
        18 => (
            P::BorderWidth,
            render(|p, v| side(&mut p.border_sides.right).width = v)?,
        ),
        19 => (
            P::BorderWidth,
            render(|p, v| side(&mut p.border_sides.bottom).width = v)?,
        ),
        20 => (
            P::BorderWidth,
            render(|p, v| side(&mut p.border_sides.left).width = v)?,
        ),
        21 => (P::BorderWidth, render(|p, v| p.outline_width = v)?),
        22 => (P::BorderWidth, render(|p, v| p.outline_offset = v)?),
        // How far in from a side a clipping box fades what it clips.
        28 => (P::Opacity, render(|p, v| p.overflow_fade.top = v)?),
        29 => (P::Opacity, render(|p, v| p.overflow_fade.right = v)?),
        30 => (P::Opacity, render(|p, v| p.overflow_fade.bottom = v)?),
        31 => (P::Opacity, render(|p, v| p.overflow_fade.left = v)?),
        // CSS's colour filters, each over the identity: 1 for brightness, contrast and saturate, 0 for the rest.
        33 => (P::Filter, render(|p, v| filter(p).brightness = v)?),
        34 => (P::Filter, render(|p, v| filter(p).contrast = v)?),
        35 => (P::Filter, render(|p, v| filter(p).grayscale = v)?),
        36 => (P::Filter, render(|p, v| filter(p).hue_rotate = v)?),
        37 => (P::Filter, render(|p, v| filter(p).invert = v)?),
        38 => (P::Filter, render(|p, v| filter(p).saturate = v)?),
        39 => (P::Filter, render(|p, v| filter(p).sepia = v)?),
        40 => (P::Filter, render(|p, v| filter(p).blur = v)?),
        // Whether text breaks lines at its width, as CSS's white-space: 0 keeps it to one line.
        48 => (
            P::TextAlign,
            render(move |_, v: f32| record_text(node, move |c| c.wrap = v != 0.0))?,
        ),
        // The backdrop's colour filters, in BACKDROP_IDENTITY's order.
        49..=55 => {
            let filter = (raw - SIDES_BASE - 49) as usize;
            (
                P::Background,
                render(move |_, v: f32| record_backdrop(node, filter, v))?,
            )
        }
        _ => return None,
    })
}

fn side(slot: &mut Option<BorderSide>) -> &mut BorderSide {
    layout_props::border_side(slot)
}

fn filter(p: &mut RenderProps) -> &mut blinc_layout::element_style::CssFilter {
    p.filter.get_or_insert_with(Default::default)
}

/// ashui's own value properties, numbered after its number ones: the
/// outline's colour, each border side's, and the clip path.
fn own_value_write(raw: i32) -> Option<(PropertyId, Write<Value>)> {
    fn color(f: fn(&mut RenderProps, blinc_core::Color)) -> Option<Write<Value>> {
        render(move |p, v| {
            if let Value::Color(c) = v {
                f(p, c);
            }
        })
    }
    Some(match raw {
        66 => (
            PropertyId::AccentColor,
            color(|p, c| p.outline_color = Some(c))?,
        ),
        67 => (
            PropertyId::BorderColor,
            color(|p, c| side(&mut p.border_sides.top).color = c)?,
        ),
        68 => (
            PropertyId::BorderColor,
            color(|p, c| side(&mut p.border_sides.right).color = c)?,
        ),
        69 => (
            PropertyId::BorderColor,
            color(|p, c| side(&mut p.border_sides.bottom).color = c)?,
        ),
        70 => (
            PropertyId::BorderColor,
            color(|p, c| side(&mut p.border_sides.left).color = c)?,
        ),
        // CSS's mask-image: a gradient whose alpha the element and what it holds are drawn through.
        89 => (
            PropertyId::Filter,
            render(|p, v| {
                p.mask_image = match v {
                    Value::Brush(blinc_core::Brush::Gradient(g)) => {
                        Some(blinc_core::MaskImage::Gradient(g))
                    }
                    _ => None,
                };
            })?,
        ),
        84 => (
            PropertyId::Filter,
            render(|p, v| {
                filter(p).drop_shadow = match v {
                    Value::Shadow(outer, _) => outer.first().copied(),
                    _ => None,
                };
            })?,
        ),
        75 => (
            PropertyId::Transform,
            render(|p, v| {
                p.clip_path = match v {
                    Value::ClipPath(c) => Some(c),
                    _ => None,
                };
            })?,
        ),
        _ => return None,
    })
}

fn f32_write(node: LayoutNodeId, prop: PropertyId) -> Option<Write<f32>> {
    use PropertyId as P;
    match prop {
        P::Opacity => render(|p, v| p.opacity = v),
        P::BorderWidth => render(|p, v| p.border_width = v),
        P::FontSize => render(move |p, v| {
            p.font_size = Some(v);
            record_text(node, move |c| c.font_size = v);
        }),
        P::LetterSpacing => render(move |p, v| {
            p.letter_spacing = Some(v);
            record_text(node, move |c| c.letter_spacing = v);
        }),
        P::LineHeight => render(move |p, v| {
            p.line_height = Some(v);
            record_text(node, move |c| c.line_height = v);
        }),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_apply_f32(
    node: u64,
    prop: i32,
    kind: i32,
    constant: f32,
    sig: *mut c_void,
    comp: *mut c_void,
) {
    let node = LayoutNodeId::from_raw(node);
    if let Some(id) = layout_props::f32_property(prop) {
        let write = Write::Layout(Arc::new(move |s: &mut Style, v| {
            layout_props::set_f32(s, prop, v);
        }));
        unsafe { bind(node, id, kind, constant, sig, comp, write) };
        return;
    }
    if let Some((prop, write)) = side_write(node, prop) {
        unsafe { bind(node, prop, kind, constant, sig, comp, write) };
        return;
    }
    let Some(prop) = property(prop) else { return };
    let Some(write) = f32_write(node, prop) else {
        return;
    };
    unsafe { bind(node, prop, kind, constant, sig, comp, write) };
}
define_prim!(
    hlp_blinc_apply_f32,
    hl_blinc_apply_f32,
    "PliifXblinc_signal_Xblinc_computed__v"
);

// ============================================================================
// I32: layout and text enums
// ============================================================================

/// A CSS numeric weight, to the nearest named one.
fn font_weight(v: i32) -> FontWeight {
    match v {
        ..=149 => FontWeight::Thin,
        150..=249 => FontWeight::ExtraLight,
        250..=349 => FontWeight::Light,
        350..=449 => FontWeight::Normal,
        450..=549 => FontWeight::Medium,
        550..=649 => FontWeight::SemiBold,
        650..=749 => FontWeight::Bold,
        750..=849 => FontWeight::ExtraBold,
        _ => FontWeight::Black,
    }
}

fn font_style(v: i32) -> FontStyle {
    match v {
        1 => FontStyle::Italic,
        _ => FontStyle::Normal,
    }
}

fn text_align(v: i32) -> TextAlign {
    match v {
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

fn i32_write(node: LayoutNodeId, prop: PropertyId) -> Option<Write<i32>> {
    use PropertyId as P;
    match prop {
        P::FontWeight => render(move |p, v| {
            p.font_weight = Some(font_weight(v));
            record_text(node, move |c| c.font_weight = v.clamp(1, 1000) as u16);
        }),
        P::FontStyle => render(move |p, v| {
            p.font_style = Some(font_style(v));
            record_text(node, move |c| c.italic = v == 1);
        }),
        P::TextAlign => render(|p, v| p.text_align = Some(text_align(v))),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_apply_i32(
    node: u64,
    prop: i32,
    kind: i32,
    constant: i32,
    sig: *mut c_void,
    comp: *mut c_void,
) {
    let node = LayoutNodeId::from_raw(node);
    if let Some(id) = layout_props::i32_property(prop) {
        let write = Write::Layout(Arc::new(move |s: &mut Style, v| {
            layout_props::set_i32(s, prop, v);
        }));
        unsafe { bind(node, id, kind, constant, sig, comp, write) };
        return;
    }
    let Some(prop) = property(prop) else { return };
    let Some(write) = i32_write(node, prop) else {
        return;
    };
    unsafe { bind(node, prop, kind, constant, sig, comp, write) };
}
define_prim!(
    hlp_blinc_apply_i32,
    hl_blinc_apply_i32,
    "PliiiXblinc_signal_Xblinc_computed__v"
);

// ============================================================================
// VALUES: brushes, colors, radii, transforms, shadows
// ============================================================================

/// A value of the wrong variant for the property is ignored.
fn value_write(node: LayoutNodeId, prop: PropertyId) -> Option<Write<Value>> {
    use PropertyId as P;
    match prop {
        P::Background => render(move |p, v| match v {
            Value::Glass(g, effects) => {
                record_glass(node, if g.simple { None } else { Some(effects) });
                p.background = Some(blinc_core::Brush::Glass(g));
            }
            Value::Brush(b) => {
                record_glass(node, None);
                p.background = Some(b);
            }
            Value::Color(c) => {
                record_glass(node, None);
                p.background = Some(c.into());
            }
            _ => {}
        }),
        P::BorderColor => render(|p, v| {
            if let Value::Color(c) = v {
                p.border_color = Some(c);
            }
        }),
        P::Color => render(|p, v| {
            if let Value::Color(c) = v {
                p.text_color = Some(c.to_array());
            }
        }),
        P::AccentColor => render(|p, v| {
            if let Value::Color(c) = v {
                p.outline_color = Some(c);
            }
        }),
        // The radius and the corner shape share this property.
        P::CornerRadius => render(|p, v| match v {
            Value::Radius(r) => {
                p.border_radius = r;
                p.border_radius_explicit = true;
            }
            Value::CornerShape(n, locked) => {
                p.corner_shape = CornerShape::new(n[0], n[1], n[2], n[3]);
                p.corner_shape_locked = locked;
            }
            _ => {}
        }),
        P::Transform => render(|p, v| {
            if let Value::Transform(t) = v {
                p.transform = Some(t);
            }
        }),
        P::Shadow => render(|p, v| {
            if let Value::Shadow(outer, inner) = v {
                p.shadow = outer;
                p.inner_shadow = inner;
            }
        }),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_apply_value(
    node: u64,
    prop: i32,
    kind: i32,
    constant: *mut c_void,
    sig: *mut c_void,
    comp: *mut c_void,
) {
    let (prop, write) = if let Some(own) = own_value_write(prop) {
        own
    } else {
        let Some(prop) = property(prop) else { return };
        let Some(write) = value_write(LayoutNodeId::from_raw(node), prop) else {
            return;
        };
        (prop, write)
    };
    let constant = unsafe { handle_ref::<Value>(constant) }
        .cloned()
        .unwrap_or_default();
    unsafe {
        bind(
            LayoutNodeId::from_raw(node),
            prop,
            kind,
            constant,
            sig,
            comp,
            write,
        )
    };
}
define_prim!(
    hlp_blinc_apply_value,
    hl_blinc_apply_value,
    "PliiXblinc_value_Xblinc_signal_Xblinc_computed__v"
);

// ============================================================================
// STRINGS
// ============================================================================

/// `TextContent` and `FontFamily`. Both live only in a text node's measure
/// context: Blinc's `RenderProps` has neither field.
fn string_write(node: LayoutNodeId, prop: PropertyId) -> Option<Write<Option<String>>> {
    use PropertyId as P;
    match prop {
        P::TextContent => render(move |_, v: Option<String>| {
            record_text(node, move |c| c.content = v.unwrap_or_default())
        }),
        P::FontFamily => render(move |_, v: Option<String>| {
            let (name, generic) = v
                .as_deref()
                .map(crate::text::resolve_family)
                .unwrap_or_default();
            record_text(node, move |c| {
                c.font_name = name;
                c.generic_font = generic;
            })
        }),
        _ => None,
    }
}

/// ashui's grid properties, written as CSS text: `GridTemplateColumns`
/// (85), `GridTemplateRows` (86), `GridColumn` (87) and `GridRow` (88).
/// Text that does not parse puts the default back.
fn own_string_write(raw: i32) -> Option<(PropertyId, Write<Option<String>>)> {
    let id = layout_props::string_property(raw)?;
    let write = Write::Layout(Arc::new(move |s: &mut Style, v: Option<String>| {
        layout_props::set_string(s, raw, v.as_deref());
    }));
    Some((id, write))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_apply_string(
    node: u64,
    prop: i32,

    kind: i32,
    constant: *const vbyte,
    sig: *mut c_void,
    comp: *mut c_void,
) {
    let node = LayoutNodeId::from_raw(node);
    let (prop, write) = if let Some(own) = own_string_write(prop) {
        own
    } else {
        let Some(prop) = property(prop) else { return };
        let Some(write) = string_write(node, prop) else {
            return;
        };
        (prop, write)
    };
    let constant = unsafe { opt_string_from(constant) };
    unsafe { bind(node, prop, kind, constant, sig, comp, write) };
}
define_prim!(
    hlp_blinc_apply_string,
    hl_blinc_apply_string,
    "PliiBXblinc_signal_Xblinc_computed__v"
);

// ============================================================================
// UNSET: a property back to what a new node has
// ============================================================================

/// Corner shapes share CornerRadius's property; this id resets the shape alone.
const CORNER_SHAPE: i32 = 1003;

/// Puts property `raw` (an ashui `PropertyId`, or `CORNER_SHAPE`) of `node`
/// back to its value on a new node: taffy's default style, Blinc's default
/// render props, and for text the measurer's defaults `Text` starts from.
/// A per-side or percentage id resets the field it writes, so `Width` and
/// `WidthPercent` reset the same one. A binding to a signal or computed is
/// not dropped; unset only what was set to a constant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_unset(node: u64, raw: i32) {
    let node = LayoutNodeId::from_raw(node);
    if let Some(prop) = layout_props::unset_property(raw) {
        queue_layout_update_partial(node, prop, prop.side_effects(), move |s| {
            layout_props::unset(s, raw);
        });
        return;
    }
    let ren = |prop: PropertyId, f: Box<dyn Fn(&mut RenderProps) + Send + Sync>| {
        queue_prop_update_partial(node, prop, prop.side_effects(), move |p| f(p))
    };
    use PropertyId as P;
    match raw {
        0 => ren(
            P::Background,
            Box::new(move |p| {
                record_glass(node, None);
                p.background = None;
            }),
        ),
        1 => ren(P::BorderColor, Box::new(|p| p.border_color = None)),
        2 => ren(
            P::BorderWidth,
            Box::new(|p| p.border_width = RenderProps::default().border_width),
        ),
        3 => ren(
            P::CornerRadius,
            Box::new(|p| {
                p.border_radius = Default::default();
                p.border_radius_explicit = false;
            }),
        ),
        CORNER_SHAPE => ren(
            P::CornerRadius,
            Box::new(|p| {
                let d = RenderProps::default();
                p.corner_shape = d.corner_shape;
                p.corner_shape_locked = d.corner_shape_locked;
            }),
        ),
        4 => ren(P::Opacity, Box::new(|p| p.opacity = 1.0)),
        5 => ren(P::Transform, Box::new(|p| p.transform = None)),
        6 => ren(
            P::Shadow,
            Box::new(|p| {
                p.shadow = Default::default();
                p.inner_shadow = Default::default();
            }),
        ),
        7 => ren(P::Color, Box::new(|p| p.text_color = None)),
        8 => ren(P::Filter, Box::new(|p| p.filter = None)),
        9 | 66 => ren(P::AccentColor, Box::new(|p| p.outline_color = None)),
        34 => ren(
            P::FontSize,
            Box::new(move |p| {
                p.font_size = None;
                record_text(node, |c| c.font_size = 16.0);
            }),
        ),
        35 => ren(
            P::FontFamily,
            Box::new(move |_| {
                record_text(node, |c| {
                    c.font_name = None;
                    c.generic_font = Default::default();
                })
            }),
        ),
        36 => ren(
            P::FontWeight,
            Box::new(move |p| {
                p.font_weight = None;
                record_text(node, |c| c.font_weight = 400);
            }),
        ),
        37 => ren(
            P::FontStyle,
            Box::new(move |p| {
                p.font_style = None;
                record_text(node, |c| c.italic = false);
            }),
        ),
        38 => ren(
            P::LetterSpacing,
            Box::new(move |p| {
                p.letter_spacing = None;
                record_text(node, |c| c.letter_spacing = 0.0);
            }),
        ),
        39 => ren(
            P::LineHeight,
            Box::new(move |p| {
                p.line_height = None;
                record_text(node, |c| c.line_height = 1.2);
            }),
        ),
        40 => ren(P::TextAlign, Box::new(|p| p.text_align = None)),
        60 => ren(
            P::BorderWidth,
            Box::new(|p| side(&mut p.border_sides.top).width = -1.0),
        ),
        61 => ren(
            P::BorderWidth,
            Box::new(|p| side(&mut p.border_sides.right).width = -1.0),
        ),
        62 => ren(
            P::BorderWidth,
            Box::new(|p| side(&mut p.border_sides.bottom).width = -1.0),
        ),
        63 => ren(
            P::BorderWidth,
            Box::new(|p| side(&mut p.border_sides.left).width = -1.0),
        ),
        64 => ren(
            P::BorderWidth,
            Box::new(|p| p.outline_width = RenderProps::default().outline_width),
        ),
        65 => ren(
            P::BorderWidth,
            Box::new(|p| p.outline_offset = RenderProps::default().outline_offset),
        ),
        67..=70 => {
            let unset = blinc_core::Color {
                r: f32::NAN,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            };
            ren(
                P::BorderColor,
                Box::new(move |p| {
                    let s = &mut p.border_sides;
                    let slot = match raw {
                        67 => &mut s.top,
                        68 => &mut s.right,
                        69 => &mut s.bottom,
                        _ => &mut s.left,
                    };
                    side(slot).color = unset;
                }),
            )
        }
        71 => ren(P::Opacity, Box::new(|p| p.overflow_fade.top = 0.0)),
        72 => ren(P::Opacity, Box::new(|p| p.overflow_fade.right = 0.0)),
        73 => ren(P::Opacity, Box::new(|p| p.overflow_fade.bottom = 0.0)),
        74 => ren(P::Opacity, Box::new(|p| p.overflow_fade.left = 0.0)),
        75 => ren(P::Transform, Box::new(|p| p.clip_path = None)),
        // Each colour filter back to its identity.
        76 => ren(P::Filter, Box::new(|p| filter(p).brightness = 1.0)),
        77 => ren(P::Filter, Box::new(|p| filter(p).contrast = 1.0)),
        78 => ren(P::Filter, Box::new(|p| filter(p).grayscale = 0.0)),
        79 => ren(P::Filter, Box::new(|p| filter(p).hue_rotate = 0.0)),
        80 => ren(P::Filter, Box::new(|p| filter(p).invert = 0.0)),
        81 => ren(P::Filter, Box::new(|p| filter(p).saturate = 1.0)),
        82 => ren(P::Filter, Box::new(|p| filter(p).sepia = 0.0)),
        83 => ren(P::Filter, Box::new(|p| filter(p).blur = 0.0)),
        84 => ren(P::Filter, Box::new(|p| filter(p).drop_shadow = None)),
        89 => ren(P::Filter, Box::new(|p| p.mask_image = None)),
        91 => ren(
            P::TextAlign,
            Box::new(move |_| record_text(node, |c| c.wrap = true)),
        ),
        92..=98 => {
            let filter = (raw - 92) as usize;
            ren(
                P::Background,
                Box::new(move |_| record_backdrop(node, filter, BACKDROP_IDENTITY[filter])),
            )
        }
        _ => {}
    }
}
define_prim!(hlp_blinc_unset, hl_blinc_unset, "li_v");
