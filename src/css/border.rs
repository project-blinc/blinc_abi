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
    /// The part that is a colour, as written.
    pub color: Option<String>,
}

impl Parsed {
    /// The width it gives: 0 for a style that draws nothing.
    pub fn drawn(&self) -> f64 {
        if self.none { 0.0 } else { self.width }
    }
}

/// `1px solid red`. Only whether there is a border is read of the style:
/// `none` draws none and every other style draws solid. An outline also
/// takes `auto`, which draws.
pub fn parse(text: &str, units: &Units, outline: bool) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        width: 3.0,
        none: false,
        color: None,
    };
    for part in split(text, ' ') {
        let p = part.to_ascii_lowercase();
        if STYLES.contains(&p.as_str()) || (outline && p == "auto") {
            parsed.none = p == "none" || (!outline && p == "hidden");
        } else if matches!(p.as_str(), "thin" | "medium" | "thick") || dimension(&p).is_some() {
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
