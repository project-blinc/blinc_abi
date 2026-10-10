//! The text decoration properties — `text-decoration` and its longhands, and
//! `text-underline-offset` — read as one value for an element, which its text
//! and the text of its descendants are drawn with.

use super::color;
use super::quantity::{PaintUnits, length};
use super::value::{dimension, split};
use blinc_core::Color;

/// The lines a decoration draws, as bits.
pub const UNDERLINE: u8 = 1;
pub const OVERLINE: u8 = 2;
pub const LINE_THROUGH: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Solid,
    Double,
    Dotted,
    Dashed,
}

/// How an element's text, and its descendants', is decorated. They stack as CSS's do: a descendant's
/// own decoration is drawn beside its ancestors', and none undoes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDecoration {
    /// [`UNDERLINE`], [`OVERLINE`] and [`LINE_THROUGH`] as bits.
    pub lines: u8,
    pub style: Style,
    /// None for the text's own colour.
    pub color: Option<Color>,
    /// Pixels; none for the font's own.
    pub thickness: Option<f32>,
    /// How far an underline is below the baseline, in pixels; none for the font's own.
    pub offset: Option<f32>,
}

/// What the shorthand names; what it leaves out is its initial value.
#[derive(Default)]
struct Parts {
    lines: Option<u8>,
    style: Option<Style>,
    color: Option<Option<Color>>,
    thickness: Option<Option<f32>>,
}

/// Whether `name` is one of the properties read here.
pub fn is_property(name: &str) -> bool {
    matches!(
        name,
        "text-decoration"
            | "text-decoration-line"
            | "text-decoration-style"
            | "text-decoration-color"
            | "text-decoration-thickness"
            | "text-underline-offset"
    )
}

/// The lines of a `text-decoration-line`: `none`, or any of the three.
pub fn line(text: &str) -> Result<u8, String> {
    let mut lines = 0u8;
    let mut none = false;
    for word in split(text, ' ') {
        lines |= match word.to_ascii_lowercase().as_str() {
            "none" => {
                none = true;
                0
            }
            "underline" => UNDERLINE,
            "overline" => OVERLINE,
            "line-through" => LINE_THROUGH,
            "blink" | "spelling-error" | "grammar-error" => {
                return Err(format!("text-decoration-line {word} is not supported"));
            }
            _ => {
                return Err(format!(
                    "expected none, underline, overline or line-through, not \"{word}\""
                ));
            }
        };
    }
    match (none, lines) {
        (true, 0) | (false, 1..) => Ok(lines),
        (true, _) => Err("none stands alone in text-decoration-line".into()),
        (false, 0) => Err("expected none, underline, overline or line-through".into()),
    }
}

pub fn style(text: &str) -> Result<Style, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "solid" => Ok(Style::Solid),
        "double" => Ok(Style::Double),
        "dotted" => Ok(Style::Dotted),
        "dashed" => Ok(Style::Dashed),
        "wavy" => Err("text-decoration-style wavy is not supported".into()),
        _ => Err(format!(
            "expected solid, double, dotted or dashed, not \"{}\"",
            text.trim()
        )),
    }
}

/// A colour, or `currentcolor` for the text's own, which is none.
pub fn colour(text: &str, units: &PaintUnits) -> Result<Option<Color>, String> {
    if text.trim().eq_ignore_ascii_case("currentcolor") {
        return Ok(None);
    }
    color::parse(text, units.color).map(Some)
}

/// `auto` or `from-font`, or a length or a percentage of the font size, in pixels.
pub fn thickness(text: &str, units: &PaintUnits) -> Result<Option<f32>, String> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("auto") || t.eq_ignore_ascii_case("from-font") {
        return Ok(None);
    }
    let px = measure(t, units)?;
    if !px.is_finite() || px < 0.0 {
        return Err(format!("expected a nonnegative thickness, not \"{t}\""));
    }
    Ok(Some(px))
}

/// `auto`, or a length or a percentage of the font size, in pixels.
pub fn offset(text: &str, units: &PaintUnits) -> Result<Option<f32>, String> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    let px = measure(t, units)?;
    if !px.is_finite() {
        return Err(format!("expected a length, not \"{t}\""));
    }
    Ok(Some(px))
}

fn measure(text: &str, units: &PaintUnits) -> Result<f32, String> {
    match dimension(text) {
        Some((v, unit)) if unit == "%" => Ok((v / 100.0 * units.font_size) as f32),
        _ => length(text, units).map(|v| v as f32),
    }
}

/// `text-decoration`: a line, a style, a colour and a thickness in any order.
fn shorthand(text: &str, units: &PaintUnits) -> Result<Parts, String> {
    let mut parts = Parts::default();
    let mut lines = Vec::new();
    for word in split(text, ' ') {
        let lower = word.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "none" | "underline" | "overline" | "line-through" | "blink"
        ) {
            lines.push(word);
        } else if matches!(
            lower.as_str(),
            "solid" | "double" | "dotted" | "dashed" | "wavy"
        ) {
            if parts.style.replace(style(&word)?).is_some() {
                return Err("text-decoration names one style".into());
            }
        } else if lower == "auto"
            || lower == "from-font"
            || dimension(&word).is_some_and(|(v, unit)| !unit.is_empty() || v == 0.0)
        {
            if parts.thickness.replace(thickness(&word, units)?).is_some() {
                return Err("text-decoration names one thickness".into());
            }
        } else if parts.color.replace(colour(&word, units)?).is_some() {
            return Err("text-decoration names one colour".into());
        }
    }
    if !lines.is_empty() {
        parts.lines = Some(line(&lines.join(" "))?);
    }
    Ok(parts)
}

/// The decoration `name` set to `value` makes of the node's declarations, the ones beside it
/// read from `units`. None for a declaration that draws nothing by itself.
pub fn compose(
    name: &str,
    value: &str,
    units: &PaintUnits,
) -> Result<Option<TextDecoration>, String> {
    // The declared `name` is the one being applied; the others are the node's.
    let of = |wanted: &str| {
        if name == wanted {
            Some(value)
        } else {
            units.declaration(wanted)
        }
    };
    let mut parts = Parts::default();
    if let Some(v) = of("text-decoration") {
        match shorthand(v, units) {
            // A shorthand that names no line, a colour alone, leaves it at none.
            Ok(mut p) => {
                p.lines.get_or_insert(0);
                parts = p;
            }
            Err(e) if name == "text-decoration" => return Err(e),
            Err(_) => {}
        }
    }
    if let Some(v) = of("text-decoration-line") {
        match line(v) {
            Ok(l) => parts.lines = Some(l),
            Err(e) if name == "text-decoration-line" => return Err(e),
            Err(_) => {}
        }
    }
    if let Some(v) = of("text-decoration-style") {
        match style(v) {
            Ok(s) => parts.style = Some(s),
            Err(e) if name == "text-decoration-style" => return Err(e),
            Err(_) => {}
        }
    }
    if let Some(v) = of("text-decoration-color") {
        match colour(v, units) {
            Ok(c) => parts.color = Some(c),
            Err(e) if name == "text-decoration-color" => return Err(e),
            Err(_) => {}
        }
    }
    if let Some(v) = of("text-decoration-thickness") {
        match thickness(v, units) {
            Ok(t) => parts.thickness = Some(t),
            Err(e) if name == "text-decoration-thickness" => return Err(e),
            Err(_) => {}
        }
    }
    let offset = match of("text-underline-offset") {
        Some(v) => match offset(v, units) {
            Ok(o) => o,
            Err(e) if name == "text-underline-offset" => return Err(e),
            Err(_) => None,
        },
        None => None,
    };
    let Some(lines) = parts.lines else {
        return Ok(None);
    };
    Ok(Some(TextDecoration {
        lines,
        style: parts.style.unwrap_or(Style::Solid),
        color: parts.color.unwrap_or(None),
        thickness: parts.thickness.unwrap_or(None),
        offset,
    }))
}
