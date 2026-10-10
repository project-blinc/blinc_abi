//! The quantities paint values are made of: percentages, angles and lengths,
//! and what relative ones are relative to.

use super::value::dimension;
use blinc_core::Color;

/// What relative lengths and `currentcolor` are relative to, and the node's
/// other declarations, for settings read together.
#[derive(Clone, Copy, Debug)]
pub struct PaintUnits<'a> {
    pub font_size: f64,
    pub root_font_size: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
    /// The node's own colour, which `currentcolor` means.
    pub color: Option<Color>,
    /// Its resolved declarations by property name.
    pub declared: &'a [(&'a str, &'a str)],
}

impl Default for PaintUnits<'_> {
    fn default() -> Self {
        PaintUnits {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: 0.0,
            viewport_height: 0.0,
            color: None,
            declared: &[],
        }
    }
}

impl PaintUnits<'_> {
    /// The node's own declaration of `name`.
    pub fn declaration(&self, name: &str) -> Option<&str> {
        self.declared
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
    }
}

/// A number or a percentage as a fraction: `50%` and `0.5` are both 0.5.
pub fn amount(text: &str) -> Result<f64, String> {
    match dimension(text) {
        Some((v, u)) if u.is_empty() => Ok(v),
        Some((v, u)) if u == "%" => Ok(v / 100.0),
        _ => Err(format!("expected a number or a percentage, not \"{text}\"")),
    }
}

/// An angle in radians: `deg`, `rad`, `grad` or `turn`; a bare 0.
pub fn angle(text: &str) -> Result<f64, String> {
    let (v, unit) = dimension(text).ok_or_else(|| format!("expected an angle, not \"{text}\""))?;
    match unit.as_str() {
        "deg" => Ok(v.to_radians()),
        "rad" => Ok(v),
        "grad" => Ok(v * std::f64::consts::PI / 200.0),
        "turn" => Ok(v * std::f64::consts::TAU),
        "" if v == 0.0 => Ok(0.0),
        _ => Err(format!(
            "expected an angle in deg, rad, grad or turn, not \"{text}\""
        )),
    }
}

/// A length in pixels. A percentage is of something the caller does not know.
pub fn length(text: &str, units: &PaintUnits) -> Result<f64, String> {
    let bad = || format!("expected a length, not \"{text}\"");
    let (v, unit) = dimension(text).ok_or_else(bad)?;
    Ok(match unit.as_str() {
        "" | "px" => v,
        "em" => v * units.font_size,
        "rem" => v * units.root_font_size,
        "vw" => v / 100.0 * units.viewport_width,
        "vh" => v / 100.0 * units.viewport_height,
        "vmin" => v / 100.0 * units.viewport_width.min(units.viewport_height),
        "vmax" => v / 100.0 * units.viewport_width.max(units.viewport_height),
        "%" => return Err("a percentage is not supported here".into()),
        _ => return Err(bad()),
    })
}

/// One to four values as a box's sides take them: top, right, bottom, left,
/// the missing ones copied as `margin` copies them.
pub fn box_sides<T: Clone>(parts: Vec<T>, name: &str) -> Result<[T; 4], String> {
    Ok(match parts.as_slice() {
        [a] => [a.clone(), a.clone(), a.clone(), a.clone()],
        [a, b] => [a.clone(), b.clone(), a.clone(), b.clone()],
        [a, b, c] => [a.clone(), b.clone(), c.clone(), b.clone()],
        [a, b, c, d] => [a.clone(), b.clone(), c.clone(), d.clone()],
        _ => return Err(format!("{name} takes one to four values")),
    })
}
