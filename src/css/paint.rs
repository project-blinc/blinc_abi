//! CSS paint declarations as typed writes: what a host draws a node with.
//! The counterpart of [`super::layout`] for paint. The values are typed, so
//! each host applies them to its own store; the cascade stays independent of
//! where paint lives.

use super::border;
use super::clip_path::{self, Clip};
use super::filter::{self, BACKDROP_IDENTITY, Filter};
use super::layout::Units;
use super::quantity::{PaintUnits, amount, box_sides, length};
use super::text_decoration::{self, TextDecoration};
use super::transform;
use super::value::{call, dimension, number, split};
use super::{color, gradient, shadow};
use crate::types::GlassEffects;
use blinc_core::layer::{Affine2D, BlurStyle, GlassStyle, Gradient, ImageFit};
use blinc_core::{Color, Shadow};

/// What fills a node's box.
#[derive(Clone, Debug)]
pub enum Background {
    None,
    Solid(Color),
    Gradient(Gradient),
    /// Blurred backdrop, with the effects that sit beside the style.
    Glass(GlassStyle, GlassEffects),
    /// What is behind the box, blurred, with the fill colour over it: the fill of a node with a
    /// `backdrop-filter`.
    Blur(BlurStyle),
    /// An image the host loads by its `source`, drawn as `fit` fills the box: `Tile` repeats it at
    /// its own size.
    Image {
        source: String,
        fit: ImageFit,
    },
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
    /// The shape the node and what it holds are clipped to; none for no clip.
    ClipPath(Option<Clip>),
    /// How far content fades out at each edge of a clipping node, top, right, bottom, left, in
    /// pixels; 0 is a hard edge.
    OverflowFade([f32; 4]),
    /// The colour filters over what is behind the node: brightness, contrast, grayscale,
    /// hue-rotate in degrees, invert, saturate, sepia.
    BackdropFilter([f32; 7]),
    /// How the node's text, and its descendants', is decorated; none for no declaration.
    TextDecoration(Option<TextDecoration>),
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
            | "clip-path"
            | "overflow-fade"
            | "background-repeat"
            | "background-position"
            | "backdrop-filter"
            | "text-decoration"
            | "text-decoration-line"
            | "text-decoration-style"
            | "text-decoration-color"
            | "text-decoration-thickness"
            | "text-underline-offset"
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
        // Read by an image background, beside it.
        ("background-size" | "background-repeat" | "background-position", _) => Ok(vec![]),
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
        ("clip-path", None) => Ok(vec![ClipPath(None)]),
        ("clip-path", Some(v)) => one(clip_path::parse(v, units).map(ClipPath)),
        ("backdrop-filter", None) => Ok(backdrop(None, units)),
        ("backdrop-filter", Some(v)) => {
            filter::parse_backdrop(v, units).map(|b| backdrop(b, units))
        }
        ("text-decoration" | "text-decoration-line", None) => Ok(vec![TextDecoration(None)]),
        // The rest are read by the line's write, beside it.
        (n, None) if text_decoration::is_property(n) => Ok(vec![]),
        (n, Some(v)) if text_decoration::is_property(n) => decoration(n, v, units),
        ("overflow-fade", None) => Ok(vec![OverflowFade([0.0; 4])]),
        ("overflow-fade", Some(v)) => one(overflow_fade(v, units).map(OverflowFade)),
        _ => return None,
    })
}

/// The writes of a `backdrop-filter`: its colour filters, and, for a node with no background of
/// its own, the blur that fills it.
fn backdrop(filter: Option<filter::Backdrop>, units: &PaintUnits) -> Vec<PaintWrite> {
    let mut writes = vec![PaintWrite::BackdropFilter(
        filter.map_or(BACKDROP_IDENTITY, |b| b.colors),
    )];
    if !has_background(units) {
        writes.push(PaintWrite::Background(blurred(
            Background::None,
            filter.map(|b| b.blur),
        )));
    }
    writes
}

fn has_background(units: &PaintUnits) -> bool {
    ["background", "background-color", "background-image"]
        .iter()
        .any(|name| units.declaration(name).is_some())
}

/// `fill` over a backdrop blurred by `blur`; a gradient, glass or image is left as it is.
fn blurred(fill: Background, blur: Option<f32>) -> Background {
    let Some(radius) = blur else {
        return fill;
    };
    let tint = match &fill {
        Background::Solid(c) => Some(*c),
        Background::None => None,
        _ => return fill,
    };
    Background::Blur(BlurStyle {
        radius,
        tint,
        ..BlurStyle::default()
    })
}

/// A `background`: `glass`, `none`, a gradient or a colour, over the blur of the node's
/// `backdrop-filter` if it has one.
fn background(value: &str, units: &PaintUnits) -> Result<Background, String> {
    let blur = units
        .declaration("backdrop-filter")
        .and_then(|v| filter::parse_backdrop(v, units).ok().flatten())
        .map(|b| b.blur);
    fill(value, units).map(|f| blurred(f, blur))
}

fn fill(value: &str, units: &PaintUnits) -> Result<Background, String> {
    let text = value.trim();
    let lower = text.to_ascii_lowercase();
    if lower == "glass" {
        return glass(units);
    }
    if lower == "none" {
        return Ok(Background::None);
    }
    if split(text, ' ')
        .first()
        .and_then(|w| call(w))
        .is_some_and(|(name, _)| name == "url")
    {
        return image(text, units);
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

/// `url(source)`, with what the shorthand adds to it and the node's `background-size`,
/// `-repeat` and `-position`: covered, contained or stretched to the box, or at its own size,
/// repeated. Anything else an image background can say is an error.
fn image(text: &str, units: &PaintUnits) -> Result<Background, String> {
    // A `/` between the position and the size may touch them: `center/cover`.
    let (head, rest) = match text.find(')') {
        Some(i) => text.split_at(i + 1),
        None => (text, ""),
    };
    let mut words = split(head, ' ');
    words.extend(split(&rest.replace('/', " / "), ' '));
    let (_, arg) = call(&words[0]).ok_or_else(|| format!("expected url(), not \"{text}\""))?;
    let source = arg.trim();
    let source = source
        .strip_prefix(['"', '\''])
        .and_then(|s| s.strip_suffix(['"', '\'']))
        .unwrap_or(source)
        .trim();
    if source.is_empty() {
        return Err("url() names no image".into());
    }
    let mut size = units.declaration("background-size").map(str::to_string);
    let mut repeat = units.declaration("background-repeat").map(str::to_string);
    let mut position = units.declaration("background-position").map(str::to_string);
    let (mut after_slash, mut size_words, mut position_words) = (false, Vec::new(), Vec::new());
    for word in &words[1..] {
        match word.to_ascii_lowercase().as_str() {
            "/" => after_slash = true,
            "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "space" | "round" => {
                repeat = Some(word.clone());
            }
            _ if after_slash => size_words.push(word.clone()),
            _ => position_words.push(word.clone()),
        }
    }
    if !size_words.is_empty() {
        size = Some(size_words.join(" "));
    }
    if !position_words.is_empty() {
        position = Some(position_words.join(" "));
    }
    if let Some(p) = &position {
        let centred = split(&p.to_ascii_lowercase(), ' ')
            .iter()
            .all(|w| matches!(w.as_str(), "center" | "50%"));
        if !centred {
            return Err(format!(
                "background-position {p}: only the centre is supported"
            ));
        }
    }
    let size = size.unwrap_or_else(|| "auto".into());
    let fit = match split(&size.to_ascii_lowercase(), ' ').join(" ").as_str() {
        "cover" => ImageFit::Cover,
        "contain" => ImageFit::Contain,
        "100% 100%" => ImageFit::Fill,
        "auto" | "auto auto" => {
            let repeat = repeat.unwrap_or_else(|| "repeat".into());
            match repeat.trim().to_ascii_lowercase().as_str() {
                "repeat" | "repeat repeat" => ImageFit::Tile,
                other => {
                    return Err(format!(
                        "background-repeat {other} is not supported for an image at its own size; \
                         tile it with repeat, or use background-size: cover, contain or 100% 100%"
                    ));
                }
            }
        }
        other => {
            return Err(format!(
                "background-size {other} is not supported; use cover, contain, 100% 100% or auto"
            ));
        }
    };
    Ok(Background::Image {
        source: source.to_string(),
        fit,
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

/// The write of one text decoration property: the decoration the node's declarations make, from
/// the shorthand or the line's, and nothing from a property that only adjusts it.
fn decoration(name: &str, value: &str, units: &PaintUnits) -> Result<Vec<PaintWrite>, String> {
    let made = text_decoration::compose(name, value, units)?;
    let writes = match name {
        // With the shorthand declared, it writes what the line adds to it.
        "text-decoration-line" if units.declaration("text-decoration").is_some() => false,
        "text-decoration" | "text-decoration-line" => true,
        _ => false,
    };
    Ok(match made {
        Some(d) if writes => vec![PaintWrite::TextDecoration(Some(d))],
        _ => vec![],
    })
}

/// `none`, or one to four distances as a box's sides take them.
fn overflow_fade(value: &str, units: &PaintUnits) -> Result<[f32; 4], String> {
    if value.trim().eq_ignore_ascii_case("none") {
        return Ok([0.0; 4]);
    }
    let mut fades = Vec::new();
    for part in box_sides(split(value, ' '), "overflow-fade")? {
        let distance = length(&part, units)?;
        if !distance.is_finite() || distance < 0.0 {
            return Err(format!("expected a nonnegative length, not \"{part}\""));
        }
        fades.push(distance as f32);
    }
    Ok([fades[0], fades[1], fades[2], fades[3]])
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
