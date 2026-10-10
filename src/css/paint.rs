//! CSS paint declarations as typed writes: what a host draws a node with.
//! The counterpart of [`super::layout`] for paint. The values are typed, so
//! each host applies them to its own store; the cascade stays independent of
//! where paint lives.

use super::border;
use super::filter::{self, Filter};
use super::layout::Units;
use super::quantity::{PaintUnits, amount, box_sides, length};
use super::transform;
use super::value::{call, dimension, number, split};
use super::{color, gradient, shadow};
use crate::types::GlassEffects;
use blinc_core::layer::{Affine2D, GlassStyle, Gradient};
use blinc_core::{Color, Shadow};

/// What fills a node's box.
#[derive(Clone, Debug)]
pub enum Background {
    None,
    Solid(Color),
    Gradient(Gradient),
    /// Blurred backdrop, with the effects that sit beside the style.
    Glass(GlassStyle, GlassEffects),
}

/// One paint field and the value it takes. Unsetting a property writes the
/// field's default.
#[derive(Clone, Debug)]
pub enum PaintWrite {
    Background(Background),
    /// The colour of the node's text; none for the inherited default.
    TextColor(Option<Color>),
    Opacity(f32),
    Visible(bool),
    /// Pixels per corner, top-left first, clockwise.
    BorderRadius([f32; 4]),
    /// Each corner's superellipse `n`, top-left first: 1 round, 2 a squircle,
    /// 0 a bevel, negative values scoop inward. `locked` keeps a round shape
    /// from being smoothed by the theme.
    CornerShape {
        shapes: [f32; 4],
        locked: bool,
    },
    /// The border's colour on every side, which takes back any side's own; none for no colour.
    BorderColor(Option<Color>),
    /// One side's colour, top 0, right 1, bottom 2, left 3; none takes the border's.
    BorderSideColor {
        side: usize,
        color: Option<Color>,
    },
    OutlineWidth(f32),
    OutlineColor(Option<Color>),
    /// The gap between the border and the outline, in pixels.
    OutlineOffset(f32),
    /// Outer and inset layers, the first drawn on top.
    Shadows {
        outer: Vec<Shadow>,
        inner: Vec<Shadow>,
    },
    Transform(Affine2D),
    Filter(Filter),
    Mask(Option<Gradient>),
}

const GLASS: &[&str] = &[
    "glass-blur",
    "glass-tint",
    "glass-aberration",
    "glass-bevel",
    "glass-noise",
    "glass-mode",
    "glass-curvature",
];

/// Whether `name` is a property [`paint_writes`] reads.
pub fn is_paint_property(name: &str) -> bool {
    matches!(
        name,
        "background"
            | "background-color"
            | "background-image"
            | "background-size"
            | "color"
            | "opacity"
            | "visibility"
            | "border-radius"
            | "corner-shape"
            | "border-color"
            | "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "outline"
            | "outline-width"
            | "outline-color"
            | "outline-offset"
            | "box-shadow"
            | "transform"
            | "filter"
            | "mask-image"
            | "-webkit-mask-image"
    ) || GLASS.contains(&name)
}

/// The writes for one CSS paint declaration, `None` value unsetting what it
/// writes; `None` for a property that is not a paint property. A value it
/// cannot read is an error giving the reason.
pub fn paint_writes(
    name: &str,
    value: Option<&str>,
    units: &PaintUnits,
) -> Option<Result<Vec<PaintWrite>, String>> {
    use PaintWrite::*;
    if !is_paint_property(name) {
        return None;
    }
    let one = |w: Result<PaintWrite, String>| w.map(|w| vec![w]);
    if name == "border" || border_side(name).is_some() {
        // A shorthand names its colour among a width and a style; a longhand is the colour.
        let side = border_side(name);
        let write = |color: Option<Color>| match side {
            Some(side) => BorderSideColor { side, color },
            None => BorderColor(color),
        };
        return Some(match value {
            None => Ok(vec![write(None)]),
            Some(v) if name.ends_with("-color") => {
                color::parse(v, units.color).map(|c| vec![write(Some(c))])
            }
            Some(v) => border::color(v, units, false).map(|c| vec![write(Some(c))]),
        });
    }
    if name == "border-color" {
        return Some(match value {
            None => Ok(vec![BorderColor(None)]),
            Some(v) => border_colors(v, units),
        });
    }
    if let Some(writes) = outline(name, value, units) {
        return Some(writes);
    }
    Some(match (name, value) {
        ("background-size", _) => Ok(vec![]),
        (n, None) if GLASS.contains(&n) => Ok(vec![]),
        (n, Some(v)) if GLASS.contains(&n) => validate_glass(n, v, units).map(|_| vec![]),
        ("background" | "background-color" | "background-image", None) => {
            Ok(vec![Background(self::Background::None)])
        }
        ("background" | "background-color" | "background-image", Some(v)) => {
            one(background(v, units).map(Background))
        }
        ("color", None) => Ok(vec![TextColor(None)]),
        ("color", Some(v)) => one(color::parse(v, units.color).map(|c| TextColor(Some(c)))),
        ("opacity", None) => Ok(vec![Opacity(1.0)]),
        ("opacity", Some(v)) => one(amount(v).map(|a| Opacity(a.clamp(0.0, 1.0) as f32))),
        ("visibility", None) => Ok(vec![Visible(true)]),
        ("visibility", Some(v)) => Ok(vec![Visible(!matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "hidden" | "collapse"
        ))]),
        ("border-radius", None) => Ok(vec![BorderRadius([0.0; 4])]),
        ("border-radius", Some(v)) => one(border_radius(v, units).map(BorderRadius)),
        ("corner-shape", None) => Ok(vec![CornerShape {
            shapes: [1.0; 4],
            locked: false,
        }]),
        ("corner-shape", Some(v)) => one(corner_shape(v)),
        ("box-shadow", None) => Ok(vec![Shadows {
            outer: vec![],
            inner: vec![],
        }]),
        ("box-shadow", Some(v)) => {
            one(shadow::parse(v, units).map(|(outer, inner)| Shadows { outer, inner }))
        }
        ("transform", None) => Ok(vec![Transform(transform::IDENTITY)]),
        ("transform", Some(v)) => one(transform::parse(v, units).map(Transform)),
        ("filter", None) => Ok(vec![Filter(self::Filter::default())]),
        ("filter", Some(v)) => one(filter::parse(v, units).map(Filter)),
        ("mask-image" | "-webkit-mask-image", None) => Ok(vec![Mask(None)]),
        ("mask-image" | "-webkit-mask-image", Some(v)) => one(mask(v, units).map(Mask)),
        _ => return None,
    })
}

/// A `background`: `glass`, `none`, a gradient or a colour.
fn background(value: &str, units: &PaintUnits) -> Result<Background, String> {
    let text = value.trim();
    let lower = text.to_ascii_lowercase();
    if lower == "glass" {
        return glass(units);
    }
    if lower == "none" {
        return Ok(Background::None);
    }
    if call(text).is_some_and(|(name, _)| name == "url") {
        return Err("url() backgrounds are not supported".into());
    }
    if gradient::is_gradient(text) {
        return Ok(Background::Gradient(
            gradient::parse(text, units.color)?.to_gradient(),
        ));
    }
    color::parse(text, units.color).map(|c| {
        if c.a == 0.0 {
            Background::None
        } else {
            Background::Solid(c)
        }
    })
}

/// The `glass-*` settings; a missing or invalid one is its default.
fn glass(units: &PaintUnits) -> Result<Background, String> {
    let setting = |name: &str, default: &str| -> String {
        match units.declaration(name) {
            Some(v) if validate_glass(name, v, units).is_ok() => v.trim().to_string(),
            _ => default.to_string(),
        }
    };
    let mut style = GlassStyle::new();
    style.blur = length(&setting("glass-blur", "12px"), units)? as f32;
    style.tint = color::parse(&setting("glass-tint", "rgba(255,255,255,0.1)"), units.color)?;
    style.simple = setting("glass-mode", "liquid").eq_ignore_ascii_case("frosted");
    style.noise = amount(&setting("glass-noise", "0"))? as f32;
    let effects = GlassEffects {
        aberration: amount(&setting("glass-aberration", "0.3"))? as f32,
        bevel: amount(&setting("glass-bevel", "1"))? as f32,
        inset: setting("glass-curvature", "outset").eq_ignore_ascii_case("inset"),
    };
    Ok(Background::Glass(style, effects))
}

fn validate_glass(name: &str, value: &str, units: &PaintUnits) -> Result<(), String> {
    match name {
        "glass-blur" => {
            let blur = length(value, units)?;
            if !blur.is_finite() || blur < 0.0 {
                return Err("expected a nonnegative length".into());
            }
        }
        "glass-tint" => {
            color::parse(value, units.color)?;
        }
        "glass-aberration" | "glass-noise" | "glass-bevel" => {
            let a = amount(value)?;
            if !a.is_finite() || !(0.0..=1.0).contains(&a) {
                return Err("expected 0 to 1, or 0% to 100%".into());
            }
        }
        "glass-mode" => {
            if !["liquid", "frosted"].contains(&value.trim().to_ascii_lowercase().as_str()) {
                return Err("expected liquid or frosted".into());
            }
        }
        "glass-curvature" => {
            if !["inset", "outset"].contains(&value.trim().to_ascii_lowercase().as_str()) {
                return Err("expected inset or outset".into());
            }
        }
        _ => {}
    }
    Ok(())
}

fn border_radius(value: &str, units: &PaintUnits) -> Result<[f32; 4], String> {
    if value.contains('/') {
        return Err("elliptical corners (a / in border-radius) are not supported".into());
    }
    let mut radii = Vec::new();
    for part in box_sides(split(value, ' '), "border-radius")? {
        if dimension(&part).is_some_and(|(_, unit)| unit == "%") {
            return Err("border-radius in % is not supported".into());
        }
        radii.push(length(&part, units)?.max(0.0) as f32);
    }
    Ok([radii[0], radii[1], radii[2], radii[3]])
}

fn corner_shape(value: &str) -> Result<PaintWrite, String> {
    let lower = value.to_ascii_lowercase();
    let words = split(&lower, ' ');
    let locked = words.iter().any(|w| w == "locked");
    let mut shapes = Vec::new();
    for word in words.iter().filter(|w| *w != "locked") {
        shapes.push(match word.as_str() {
            "round" => 1.0,
            "squircle" => 2.0,
            "bevel" => 0.0,
            "scoop" => -1.0,
            "notch" => -100.0,
            "square" => 100.0,
            _ => {
                let (_, arg) = call(word).filter(|(n, _)| n == "superellipse").ok_or_else(|| {
                    format!(
                        "expected round, squircle, bevel, scoop, notch, square or superellipse(n), not \"{word}\""
                    )
                })?;
                number(arg.trim())? as f32
            }
        });
    }
    if shapes.is_empty() || shapes.len() > 4 {
        return Err("corner-shape takes one to four shapes".into());
    }
    let corners = box_sides(shapes, "corner-shape")?;
    Ok(PaintWrite::CornerShape {
        shapes: corners,
        locked,
    })
}

/// The side a border shorthand or colour longhand is for, by index from top; none for `border`.
fn border_side(name: &str) -> Option<usize> {
    Some(match name {
        "border-top" | "border-top-color" => 0,
        "border-right" | "border-right-color" => 1,
        "border-bottom" | "border-bottom-color" => 2,
        "border-left" | "border-left-color" => 3,
        _ => return None,
    })
}

fn layout_units(units: &PaintUnits) -> Units {
    Units {
        font_size: units.font_size,
        root_font_size: units.root_font_size,
        viewport_width: units.viewport_width,
        viewport_height: units.viewport_height,
    }
}

/// `border-color`: one colour for every side, or one to four for the sides.
fn border_colors(value: &str, units: &PaintUnits) -> Result<Vec<PaintWrite>, String> {
    let parts = split(value, ' ');
    if parts.len() == 1 {
        return Ok(vec![PaintWrite::BorderColor(Some(color::parse(
            &parts[0],
            units.color,
        )?))]);
    }
    let sides = box_sides(parts, "border-color")?;
    let mut writes = Vec::new();
    for (side, part) in sides.iter().enumerate() {
        writes.push(PaintWrite::BorderSideColor {
            side,
            color: Some(color::parse(part, units.color)?),
        });
    }
    Ok(writes)
}

/// The `outline` shorthand and its longhands.
fn outline(
    name: &str,
    value: Option<&str>,
    units: &PaintUnits,
) -> Option<Result<Vec<PaintWrite>, String>> {
    use PaintWrite::*;
    Some(match (name, value) {
        ("outline", None) => Ok(vec![OutlineWidth(0.0), OutlineColor(None)]),
        ("outline", Some(v)) => {
            let layout = layout_units(units);
            border::parse(v, &layout, true).and_then(|p| {
                border::color(v, units, true)
                    .map(|c| vec![OutlineWidth(p.drawn() as f32), OutlineColor(Some(c))])
            })
        }
        ("outline-width", None) => Ok(vec![OutlineWidth(0.0)]),
        ("outline-width", Some(v)) => {
            border::width(v, &layout_units(units)).map(|w| vec![OutlineWidth(w as f32)])
        }
        ("outline-color", None) => Ok(vec![OutlineColor(None)]),
        ("outline-color", Some(v)) => {
            color::parse(v, units.color).map(|c| vec![OutlineColor(Some(c))])
        }
        ("outline-offset", None) => Ok(vec![OutlineOffset(0.0)]),
        ("outline-offset", Some(v)) => length(v, units).map(|o| vec![OutlineOffset(o as f32)]),
        _ => return None,
    })
}

/// A `mask-image`: a gradient the node is drawn through, or `none`.
fn mask(value: &str, units: &PaintUnits) -> Result<Option<Gradient>, String> {
    if value.trim().eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if !gradient::is_gradient(value) {
        return Err(format!("expected a gradient or none, not \"{value}\""));
    }
    Ok(Some(gradient::parse(value, units.color)?.to_gradient()))
}
