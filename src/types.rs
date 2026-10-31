//! Shared immutable style values. Host handles live in the compatibility module.
use blinc_core::layer::{GlassStyle, Shadow};
use blinc_core::{Brush, Color, CornerRadius, Transform};
#[derive(Clone, Default)]
pub enum Value {
    #[default]
    None,
    Brush(Brush),
    /// Blinc image brush with optional positioning and independent repeat axes.
    Image(blinc_core::layer::ImageBrush, ImageMapping),
    /// Glass and the rim settings which Blinc's GlassStyle does not carry.
    Glass(GlassStyle, GlassEffects),
    Color(Color),
    Radius(CornerRadius),
    Transform(Transform),
    /// A box shadow's layers: those cast outside the box, and the inset ones, each in CSS order.
    Shadow(Vec<Shadow>, Vec<Shadow>),
    /// A corner shape's `n` per corner, and whether the theme may not smooth it.
    CornerShape([f32; 4], bool),
    /// A CSS `clip-path` shape, and whether a polygon or path of it fills by
    /// the even-odd rule rather than nonzero.
    ClipPath(blinc_core::ClipPath, bool),
    /// How an element's text, and its descendants', is decorated.
    TextDecoration(crate::css::text_decoration::TextDecoration),
}

/// Image position as free-space fractions and pixel offsets; repeat uses bit 0 for X and bit 1 for Y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageMapping {
    pub position: [f32; 4],
    pub repeat: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct GlassEffects {
    pub aberration: f32,
    pub bevel: f32,
    pub inset: bool,
}

impl Default for GlassEffects {
    fn default() -> Self {
        Self {
            aberration: 0.3,
            bevel: 1.0,
            inset: false,
        }
    }
}

/// `0xRRGGBB` plus a separate alpha, as the Haxe API spells colors.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn hex_color(hex: i32, alpha: f32) -> Color {
    Color::from_hex(hex as u32).with_alpha(alpha)
}

/// A `clip-path` shape. `kind`: 0 circle (radius, cx, cy), 1 ellipse (rx, ry, cx, cy), 2 inset (top, right,
/// bottom, left), 3 rect (top, right, bottom, left, from the top-left), 4 xywh (x, y, w, h). `values` holds
/// the lengths in that order; bit `i` of `percent` makes length `i` a percentage, and bit `i` of `none`
/// leaves it unset (a radius then reaches the closest side). A negative `round` is no rounding; a negative
/// `kind` is no clip at all.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn clip_path_value(
    kind: i32,
    values: &[f32],
    percent: i32,
    none: i32,
    round: f32,
) -> Value {
    use blinc_core::{ClipLength, ClipPath};
    let v = |i: usize| -> f32 { values.get(i).copied().unwrap_or(0.0) };
    let len = |i: usize| -> ClipLength {
        if percent & (1 << i) != 0 {
            ClipLength::Percent(v(i))
        } else {
            ClipLength::Px(v(i))
        }
    };
    let opt = |i: usize| -> Option<ClipLength> { (none & (1 << i) == 0).then(|| len(i)) };
    let round = (round >= 0.0).then_some(round);
    // `none`: no clip, for a state that takes one away.
    if kind < 0 {
        return Value::None;
    }
    let path = match kind {
        0 => ClipPath::Circle {
            radius: opt(0),
            center: (len(1), len(2)),
        },
        1 => ClipPath::Ellipse {
            rx: opt(0),
            ry: opt(1),
            center: (len(2), len(3)),
        },
        2 => ClipPath::Inset {
            top: len(0),
            right: len(1),
            bottom: len(2),
            left: len(3),
            round,
        },
        3 => ClipPath::Rect {
            top: len(0),
            right: len(1),
            bottom: len(2),
            left: len(3),
            round,
        },
        _ => ClipPath::Xywh {
            x: len(0),
            y: len(1),
            w: len(2),
            h: len(3),
            round,
        },
    };
    Value::ClipPath(path, false)
}

/// A polygon clip path of `count` points, x and y in `values`; entry `k` of `percent`, when it has one,
/// makes value `k` a percentage. A `path`'s points are pixels, with its rings apart by a point at 1e30.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn clip_polygon_value(values: &[f32], percent: &[u8], count: i32, path: bool) -> Value {
    use blinc_core::{ClipLength, ClipPath};
    let n = count.max(0) as usize;
    let v = |k: usize| values.get(k).copied().unwrap_or(0.0);
    let len = |k: usize| {
        if percent.get(k).is_some_and(|&p| p != 0) {
            ClipLength::Percent(v(k))
        } else {
            ClipLength::Px(v(k))
        }
    };
    let clip = if values.is_empty() {
        ClipPath::Polygon { points: Vec::new() }
    } else if path {
        ClipPath::Path {
            vertices: (0..n).map(|i| (v(2 * i), v(2 * i + 1))).collect(),
        }
    } else {
        ClipPath::Polygon {
            points: (0..n).map(|i| (len(2 * i), len(2 * i + 1))).collect(),
        }
    };
    Value::ClipPath(clip, false)
}

/// `lines` is UNDERLINE | OVERLINE | LINE_THROUGH as bits; `style` is 0 solid, 1 double, 2 dotted, 3 dashed.
/// A negative alpha is no colour of its own, a negative thickness or a NaN offset none.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn text_decoration_value(
    lines: i32,
    style: i32,
    hex: i32,
    alpha: f32,
    thickness: f32,
    offset: f32,
) -> Value {
    use crate::css::text_decoration::Style;
    let style = match style {
        1 => Style::Double,
        2 => Style::Dotted,
        3 => Style::Dashed,
        _ => Style::Solid,
    };
    Value::TextDecoration(crate::css::text_decoration::TextDecoration {
        lines: lines as u8,
        style,
        color: (alpha >= 0.0).then(|| hex_color(hex, alpha)),
        thickness: (!thickness.is_nan()).then_some(thickness),
        offset: (!offset.is_nan()).then_some(offset),
    })
}

/// Sets the glass effects of `value`, a glass brush: the aberration, 0 to 1.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn glass_aberration(value: &mut Value, strength: f32) {
    if let Value::Glass(_, effects) = value {
        effects.aberration = if strength.is_finite() {
            strength.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
}

/// Sets the bevel of `value`, a glass brush; zero removes it, and `inset` reverses its curvature.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn glass_bevel(value: &mut Value, strength: f32, inset: bool) {
    if let Value::Glass(_, effects) = value {
        effects.bevel = if strength.is_finite() {
            strength.clamp(0.0, 1.0)
        } else {
            1.0
        };
        effects.inset = inset;
    }
}

/// Adds numeric mapping to an image value before it is bound to a node.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn image_mapping(value: &mut Value, x: f32, y: f32, ox: f32, oy: f32, repeat: i32) {
    let position = [x, y, ox, oy];
    if !(0..=3).contains(&repeat) || !position.iter().all(|v| v.is_finite()) {
        return;
    }
    let image = match std::mem::take(value) {
        Value::Brush(Brush::Image(image)) | Value::Image(image, _) => image,
        other => {
            *value = other;
            return;
        }
    };
    *value = Value::Image(
        image,
        ImageMapping {
            position,
            repeat: repeat as u8,
        },
    );
}

/// Adds a stop at `offset`, 0 to 1, to `value`, a gradient brush.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn gradient_stop(value: &mut Value, offset: f32, hex: i32, alpha: f32) {
    use blinc_core::layer::{Gradient, GradientStop};
    if let Value::Brush(Brush::Gradient(g)) = value {
        let stops = match g {
            Gradient::Linear { stops, .. }
            | Gradient::Radial { stops, .. }
            | Gradient::Conic { stops, .. } => stops,
        };
        stops.push(GradientStop {
            offset,
            color: hex_color(hex, alpha),
        });
    }
}

/// One layer of a shadow.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn shadow_layer(
    offset_x: f32,
    offset_y: f32,
    blur: f32,
    spread: f32,
    hex: i32,
    alpha: f32,
) -> Shadow {
    Shadow {
        offset_x,
        offset_y,
        blur,
        spread,
        color: hex_color(hex, alpha),
    }
}

/// Adds a layer to `value`, a shadow value; layers are drawn last first.
#[allow(clippy::too_many_arguments)]
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn shadow_push(
    value: &mut Value,
    offset_x: f32,
    offset_y: f32,
    blur: f32,
    spread: f32,
    hex: i32,
    alpha: f32,
    inset: bool,
) {
    if let Value::Shadow(outer, inner) = value {
        let layer = shadow_layer(offset_x, offset_y, blur, spread, hex, alpha);
        if inset {
            inner.push(layer)
        } else {
            outer.push(layer)
        }
    }
}

/// A shadow of one layer, cast inside the box when `inset`.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn shadow_value(
    offset_x: f32,
    offset_y: f32,
    blur: f32,
    spread: f32,
    hex: i32,
    alpha: f32,
    inset: bool,
) -> Value {
    let layer = shadow_layer(offset_x, offset_y, blur, spread, hex, alpha);
    if inset {
        Value::Shadow(vec![], vec![layer])
    } else {
        Value::Shadow(vec![layer], vec![])
    }
}

/// A gradient with no stops yet: linear from `(x1, y1)` to `(x2, y2)`, or radial about `(x1, y1)` of radius
/// `x2`. With `bbox` the coordinates are fractions of the box it fills, else pixels from its corner.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn gradient_value(
    radial: bool,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    bbox: bool,
) -> Value {
    use blinc_core::layer::{Gradient, GradientSpace, GradientSpread, Point};
    let space = if bbox {
        GradientSpace::ObjectBoundingBox
    } else {
        GradientSpace::UserSpace
    };
    let grad = if radial {
        Gradient::Radial {
            center: Point::new(x1, y1),
            radius: x2,
            focal: None,
            stops: Vec::new(),
            space,
            spread: GradientSpread::Pad,
        }
    } else {
        Gradient::Linear {
            start: Point::new(x1, y1),
            end: Point::new(x2, y2),
            stops: Vec::new(),
            space,
            spread: GradientSpread::Pad,
        }
    };
    Value::Brush(Brush::Gradient(grad))
}

/// A glass brush.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn glass_value(
    blur: f32,
    tint_hex: i32,
    tint_alpha: f32,
    simple: bool,
    noise: f32,
) -> Value {
    let glass = GlassStyle::new()
        .blur(blur)
        .tint(hex_color(tint_hex, tint_alpha))
        .with_simple(simple)
        .noise(noise);
    Value::Glass(glass, GlassEffects::default())
}

/// A backdrop blur brush, tinted when `tint_alpha` is above zero.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn blur_value(radius: f32, tint_hex: i32, tint_alpha: f32) -> Value {
    let mut style = blinc_core::layer::BlurStyle::with_radius(radius);
    if tint_alpha > 0.0 {
        style.tint = Some(hex_color(tint_hex, tint_alpha));
    }
    Value::Brush(Brush::Blur(style))
}

/// An image brush for `src`; `fit` 0 cover, 1 contain, 2 fill, 3 tile.
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn image_value(src: String, fit: i32) -> Value {
    use blinc_core::layer::{ImageBrush, ImageFit};
    let mut img = ImageBrush::new(src);
    img.fit = match fit {
        1 => ImageFit::Contain,
        2 => ImageFit::Fill,
        3 => ImageFit::Tile,
        _ => ImageFit::Cover,
    };
    Value::Brush(Brush::Image(img))
}

/// A two-colour linear gradient brush.
#[allow(clippy::too_many_arguments)]
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) fn linear_gradient_value(
    sx: f32,
    sy: f32,
    ex: f32,
    ey: f32,
    from_hex: i32,
    from_alpha: f32,
    to_hex: i32,
    to_alpha: f32,
) -> Value {
    use blinc_core::layer::{Gradient, Point};
    Value::Brush(Brush::Gradient(Gradient::linear(
        Point::new(sx, sy),
        Point::new(ex, ey),
        hex_color(from_hex, from_alpha),
        hex_color(to_hex, to_alpha),
    )))
}

#[cfg(feature = "hashlink")]
mod host {
    use super::*;
    use crate::hl::{handle_mut, handle_ref, into_handle, string_from};
    use hl_abi::{define_prim, vbyte};
    use std::ffi::c_void;
    fn value(v: Value) -> *mut c_void {
        into_handle(v)
    }

    /// A `clip-path` shape. `kind`: 0 circle (radius, cx, cy), 1 ellipse (rx,
    /// ry, cx, cy), 2 inset (top, right, bottom, left), 3 rect (top, right,
    /// bottom, left, from the top-left), 4 xywh (x, y, w, h). `values` holds
    /// the lengths in that order; bit `i` of `percent` makes length `i` a
    /// percentage, and bit `i` of `none` leaves it unset (a radius then reaches
    /// the closest side). A negative `round` is no rounding; a negative `kind`
    /// is no clip at all.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_clip_path(
        kind: i32,
        values: *const vbyte,
        percent: i32,
        none: i32,
        round: f32,
    ) -> *mut c_void {
        let floats: Vec<f32> = if values.is_null() {
            Vec::new()
        } else {
            (0..4)
                .map(|i| unsafe { (values as *const f32).add(i).read_unaligned() })
                .collect()
        };
        value(clip_path_value(kind, &floats, percent, none, round))
    }
    define_prim!(
        hlp_blinc_clip_path,
        hl_blinc_clip_path,
        "PiBiif_Xblinc_value_"
    );

    /// A polygon clip path of `count` points, x and y in `values`; byte `k` of
    /// `percent`, when given, makes value `k` a percentage. A `path`'s points
    /// are pixels, with its rings apart by a point at 1e30.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_clip_polygon(
        values: *const vbyte,
        percent: *const vbyte,
        count: i32,
        path: bool,
    ) -> *mut c_void {
        let n = count.max(0) as usize;
        let floats: Vec<f32> = if values.is_null() {
            Vec::new()
        } else {
            (0..2 * n)
                .map(|i| unsafe { (values as *const f32).add(i).read_unaligned() })
                .collect()
        };
        let flags: Vec<u8> = if percent.is_null() {
            Vec::new()
        } else {
            (0..2 * n)
                .map(|k| unsafe { *percent.add(k) })
                .collect()
        };
        value(clip_polygon_value(&floats, &flags, count, path))
    }
    define_prim!(
        hlp_blinc_clip_polygon,
        hl_blinc_clip_polygon,
        "PBBib_Xblinc_value_"
    );

    /// `clip`, a clip-path value, filling by the even-odd rule: a new value.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_clip_even_odd(clip: *mut c_void) -> *mut c_void {
        match unsafe { handle_ref::<Value>(clip) } {
            Some(Value::ClipPath(path, _)) => value(Value::ClipPath(path.clone(), true)),
            _ => std::ptr::null_mut(),
        }
    }
    define_prim!(
        hlp_blinc_clip_even_odd,
        hl_blinc_clip_even_odd,
        "PXblinc_value__Xblinc_value_"
    );

    // --- Brushes ---

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_brush_solid(hex: i32, alpha: f32) -> *mut c_void {
        value(Value::Brush(Brush::Solid(hex_color(hex, alpha))))
    }
    define_prim!(
        hlp_blinc_brush_solid,
        hl_blinc_brush_solid,
        "Pif_Xblinc_value_"
    );

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_brush_glass(
        blur: f32,
        tint_hex: i32,
        tint_alpha: f32,
        simple: i32,
        noise: f32,
    ) -> *mut c_void {
        value(glass_value(blur, tint_hex, tint_alpha, simple != 0, noise))
    }
    define_prim!(
        hlp_blinc_brush_glass,
        hl_blinc_brush_glass,
        "Pfififf_Xblinc_value_"
    );

    /// Set while constructing the brush, before it is assigned to a node or signal.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_brush_glass_aberration(brush: *mut c_void, strength: f32) {
        if let Some(v) = unsafe { handle_mut::<Value>(brush) } {
            glass_aberration(v, strength);
        }
    }
    define_prim!(
        hlp_blinc_brush_glass_aberration,
        hl_blinc_brush_glass_aberration,
        "PXblinc_value_f_v"
    );

    /// Set while constructing the brush; zero removes the bevel, and inset reverses its curvature.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_brush_glass_bevel(
        brush: *mut c_void,
        strength: f32,
        inset: bool,
    ) {
        if let Some(v) = unsafe { handle_mut::<Value>(brush) } {
            glass_bevel(v, strength, inset);
        }
    }
    define_prim!(
        hlp_blinc_brush_glass_bevel,
        hl_blinc_brush_glass_bevel,
        "PXblinc_value_fb_v"
    );

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_brush_blur(
        radius: f32,
        tint_hex: i32,
        tint_alpha: f32,
    ) -> *mut c_void {
        value(blur_value(radius, tint_hex, tint_alpha))
    }
    define_prim!(
        hlp_blinc_brush_blur,
        hl_blinc_brush_blur,
        "Pfif_Xblinc_value_"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_brush_image(src: *const vbyte, fit: i32) -> *mut c_void {
        value(image_value(unsafe { string_from(src) }, fit))
    }
    define_prim!(
        hlp_blinc_brush_image,
        hl_blinc_brush_image,
        "PBi_Xblinc_value_"
    );

    /// Adds numeric mapping to an image value before it is bound to a node.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_brush_image_mapping(
        brush: *mut c_void,
        x: f32,
        y: f32,
        ox: f32,
        oy: f32,
        repeat: i32,
    ) {
        if let Some(v) = unsafe { handle_mut::<Value>(brush) } {
            image_mapping(v, x, y, ox, oy, repeat);
        }
    }
    define_prim!(
        hlp_blinc_brush_image_mapping,
        hl_blinc_brush_image_mapping,
        "PXblinc_value_ffffi_v"
    );

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_brush_linear_gradient(
        sx: f32,
        sy: f32,
        ex: f32,
        ey: f32,
        from_hex: i32,
        from_alpha: f32,
        to_hex: i32,
        to_alpha: f32,
    ) -> *mut c_void {
        value(linear_gradient_value(
            sx, sy, ex, ey, from_hex, from_alpha, to_hex, to_alpha,
        ))
    }
    define_prim!(
        hlp_blinc_brush_linear_gradient,
        hl_blinc_brush_linear_gradient,
        "Pffffifif_Xblinc_value_"
    );

    /// A gradient with no stops yet: linear from `(x1, y1)` to `(x2, y2)`, or
    /// radial (`radial`) about `(x1, y1)` of radius `x2`. With `bbox` the
    /// coordinates are fractions of the box it fills, else pixels from its
    /// corner. `blinc_brush_gradient_stop` adds the stops, in order.
    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_brush_gradient(
        radial: bool,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        bbox: bool,
    ) -> *mut c_void {
        value(gradient_value(radial, x1, y1, x2, y2, bbox))
    }
    define_prim!(
        hlp_blinc_brush_gradient,
        hl_blinc_brush_gradient,
        "Pbffffb_Xblinc_value_"
    );

    /// Adds a stop at `offset`, 0 to 1, to `brush`, a gradient brush.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_brush_gradient_stop(
        brush: *mut c_void,
        offset: f32,
        hex: i32,
        alpha: f32,
    ) {
        if let Some(v) = unsafe { handle_mut::<Value>(brush) } {
            gradient_stop(v, offset, hex, alpha);
        }
    }
    define_prim!(
        hlp_blinc_brush_gradient_stop,
        hl_blinc_brush_gradient_stop,
        "PXblinc_value_fif_v"
    );

    // --- Colors ---

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_color_hex(hex: i32, alpha: f32) -> *mut c_void {
        value(Value::Color(hex_color(hex, alpha)))
    }
    define_prim!(hlp_blinc_color_hex, hl_blinc_color_hex, "Pif_Xblinc_value_");

    // --- Corner radius ---

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_corner_radius(tl: f32, tr: f32, br: f32, bl: f32) -> *mut c_void {
        value(Value::Radius(CornerRadius::new(tl, tr, br, bl)))
    }
    define_prim!(
        hlp_blinc_corner_radius,
        hl_blinc_corner_radius,
        "Pffff_Xblinc_value_"
    );

    // --- Transforms ---

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_transform_identity() -> *mut c_void {
        value(Value::Transform(Transform::identity()))
    }
    define_prim!(
        hlp_blinc_transform_identity,
        hl_blinc_transform_identity,
        "P_Xblinc_value_"
    );

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_transform_translate(x: f32, y: f32) -> *mut c_void {
        value(Value::Transform(Transform::translate(x, y)))
    }
    define_prim!(
        hlp_blinc_transform_translate,
        hl_blinc_transform_translate,
        "Pff_Xblinc_value_"
    );

    /// The 2D affine `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`.
    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_transform_affine(
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        tx: f32,
        ty: f32,
    ) -> *mut c_void {
        value(Value::Transform(Transform::Affine2D(
            blinc_core::Affine2D {
                elements: [a, b, c, d, tx, ty],
            },
        )))
    }
    define_prim!(
        hlp_blinc_transform_affine,
        hl_blinc_transform_affine,
        "Pffffff_Xblinc_value_"
    );

    // --- Shadows ---

    /// Each corner's superellipse `n`, top-left first; `locked` keeps it from
    /// the theme's squircle.
    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_corner_shape(
        top_left: f32,
        top_right: f32,
        bottom_right: f32,
        bottom_left: f32,
        locked: bool,
    ) -> *mut c_void {
        value(Value::CornerShape(
            [top_left, top_right, bottom_right, bottom_left],
            locked,
        ))
    }
    define_prim!(
        hlp_blinc_corner_shape,
        hl_blinc_corner_shape,
        "Pffffb_Xblinc_value_"
    );

    /// A shadow of one layer, cast inside the box when `inset`;
    /// `blinc_shadow_push` adds more.
    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_shadow(
        offset_x: f32,
        offset_y: f32,
        blur: f32,
        spread: f32,
        hex: i32,
        alpha: f32,
        inset: bool,
    ) -> *mut c_void {
        value(shadow_value(
            offset_x, offset_y, blur, spread, hex, alpha, inset,
        ))
    }
    define_prim!(hlp_blinc_shadow, hl_blinc_shadow, "Pffffifb_Xblinc_value_");

    /// Adds a layer to `shadow`, a shadow value; layers are drawn last first.
    /// Values already bound to a node keep the layers they had.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_shadow_push(
        shadow: *mut c_void,
        offset_x: f32,
        offset_y: f32,
        blur: f32,
        spread: f32,
        hex: i32,
        alpha: f32,
        inset: bool,
    ) {
        if let Some(v) = unsafe { handle_mut::<Value>(shadow) } {
            shadow_push(v, offset_x, offset_y, blur, spread, hex, alpha, inset);
        }
    }
    define_prim!(
        hlp_blinc_shadow_push,
        hl_blinc_shadow_push,
        "PXblinc_value_ffffifb_v"
    );

    // --- Text decoration ---

    /// `lines` is UNDERLINE | OVERLINE | LINE_THROUGH as bits; `style` is 0 solid, 1 double, 2
    /// dotted, 3 dashed. `has_color`, `has_thickness` and `has_offset` false leave the matching
    /// value at the font's own.
    #[unsafe(no_mangle)]
    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn hl_blinc_text_decoration(
        lines: i32,
        style: i32,
        hex: i32,
        alpha: f32,
        thickness: f32,
        offset: f32,
    ) -> *mut c_void {
        value(text_decoration_value(
            lines, style, hex, alpha, thickness, offset,
        ))
    }
    define_prim!(
        hlp_blinc_text_decoration,
        hl_blinc_text_decoration,
        "Piiifff_Xblinc_value_"
    );
}
#[cfg(feature = "hashlink")]
pub use host::*;
