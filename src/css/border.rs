//! The grammar border and outline shorthands share: a width, a style and a
//! colour, in any order. Layout reads the width and paint the colour, so the
//! two cannot disagree about what a shorthand says.

use super::color;
use super::layout::Units;
use super::quantity::{PaintUnits, length};
use super::value::{dimension, split};
use blinc_core::Color;

const STYLES: &[&str] = &[
    "none", "hidden", "solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset",
];

/// The patterns shared by border and text-decoration drawing. Dimensions are
/// multiples of the line width, in layout units.
#[derive(Clone, Copy, Debug)]
pub struct Pattern {
    pub on: f32,
    pub off: f32,
    pub radius: f32,
}
impl Pattern {
    pub fn dotted(width: f32) -> Self {
        Self {
            on: width,
            off: width,
            radius: width / 2.0,
        }
    }
    pub fn dashed(width: f32) -> Self {
        Self {
            on: 3.0 * width,
            off: 2.0 * width,
            radius: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    None,
    #[default]
    Solid,
    Dashed,
    Dotted,
}
impl Style {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "none" | "hidden" => Ok(Self::None),
            "dashed" => Ok(Self::Dashed),
            "dotted" => Ok(Self::Dotted),
            "solid" | "double" | "groove" | "ridge" | "inset" | "outset" => Ok(Self::Solid),
            _ => Err(format!("invalid border style: {text}")),
        }
    }
    pub fn pattern(self, width: f32) -> Option<Pattern> {
        match self {
            Self::Dashed => Some(Pattern::dashed(width)),
            Self::Dotted => Some(Pattern::dotted(width)),
            _ => None,
        }
    }
}

/// A width: `thin`, `medium` or `thick`, or a length of at least 0.
pub fn width(text: &str, units: &Units) -> Result<f64, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "thin" => Ok(1.0),
        "medium" => Ok(3.0),
        "thick" => Ok(5.0),
        t => {
            let pixels = PaintUnits {
                font_size: units.font_size,
                root_font_size: units.root_font_size,
                viewport_width: units.viewport_width,
                viewport_height: units.viewport_height,
                ..Default::default()
            };
            let w = length(t, &pixels).map_err(|_| format!("expected a width, not \"{text}\""))?;
            if w < 0.0 {
                return Err("a width cannot be negative".into());
            }
            Ok(w)
        }
    }
}

/// What a shorthand says.
pub struct Parsed {
    /// Pixels; 3 when it names none.
    pub width: f64,
    /// Whether its style draws nothing.
    pub none: bool,
    pub style: Style,
    /// The part that is a colour, as written.
    pub color: Option<String>,
}

impl Parsed {
    /// The width it gives: 0 for a style that draws nothing.
    pub fn drawn(&self) -> f64 {
        if self.none { 0.0 } else { self.width }
    }
}

/// `1px solid red`. Dashed and dotted retain their patterns; other visible
/// styles currently draw solid. An outline also takes `auto`, which draws.
pub fn parse(text: &str, units: &Units, outline: bool) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        width: 3.0,
        none: false,
        style: Style::Solid,
        color: None,
    };
    for part in split(text, ' ') {
        let p = part.to_ascii_lowercase();
        if STYLES.contains(&p.as_str()) || (outline && p == "auto") {
            parsed.none = p == "none" || (!outline && p == "hidden");
            parsed.style = Style::parse(&p).unwrap_or(Style::Solid);
        } else if matches!(p.as_str(), "thin" | "medium" | "thick")
            || dimension(&p).is_some()
            || super::calc::is_function(&p)
        {
            parsed.width = width(&p, units)?;
        } else {
            parsed.color = Some(part);
        }
    }
    Ok(parsed)
}

/// The colour a shorthand names, else the node's own, else black.
pub fn color(text: &str, units: &PaintUnits, outline: bool) -> Result<Color, String> {
    let layout = Units {
        font_size: units.font_size,
        root_font_size: units.root_font_size,
        viewport_width: units.viewport_width,
        viewport_height: units.viewport_height,
    };
    match parse(text, &layout, outline)?.color {
        Some(c) => color::parse(&c, units.color),
        None => Ok(units.color.unwrap_or(Color::rgba(0.0, 0.0, 0.0, 1.0))),
    }
}

/// Styles in cascade order, after shorthands and stronger side overrides.
/// Layout uses these after width writes: `none` has zero used width even
/// when a separate `border-width` declaration follows it.
pub fn resolved_styles<'a>(
    declared: impl Iterator<Item = (&'a str, &'a str)>,
    units: &Units,
) -> [Style; 4] {
    let mut styles = [Style::Solid; 4];
    for (name, value) in declared {
        let side = match name {
            "border-top" | "border-top-style" => Some(0),
            "border-right" | "border-right-style" => Some(1),
            "border-bottom" | "border-bottom-style" => Some(2),
            "border-left" | "border-left-style" => Some(3),
            _ => None,
        };
        if name == "border-style" {
            if let Ok(s) = split(value, ' ')
                .iter()
                .map(|v| Style::parse(v))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|s| super::quantity::box_sides(s, name))
            {
                styles = s;
            }
        } else if name == "border" || side.is_some() {
            let style = if name.ends_with("-style") {
                Style::parse(value)
            } else {
                parse(value, units, false).map(|b| b.style)
            };
            if let Ok(style) = style {
                if let Some(side) = side {
                    styles[side] = style;
                } else {
                    styles = [style; 4];
                }
            }
        }
    }
    styles
}
