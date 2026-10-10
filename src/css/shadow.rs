//! `box-shadow` and `drop-shadow()`: offsets, a blur, a spread and a colour.

use super::color;
use super::quantity::{PaintUnits, length};
use super::value::{call, dimension, split};
use blinc_core::Shadow;

/// One layer of a shadow list, and whether it is `inset`. A list with the
/// first drawn on top, as CSS writes it.
pub fn parse_one(
    text: &str,
    allow_inset: bool,
    units: &PaintUnits,
) -> Result<(bool, Shadow), String> {
    let mut inset = false;
    let mut lengths = Vec::new();
    let mut shade = None;
    for word in split(text, ' ') {
        if word.eq_ignore_ascii_case("inset") {
            if !allow_inset {
                return Err("inset is for box-shadow".into());
            }
            inset = true;
        } else if dimension(&word).is_some() || call(&word).is_some_and(|(n, _)| n == "calc") {
            lengths.push(length(&word, units)?);
        } else {
            shade = Some(color::parse(&word, units.color)?);
        }
    }
    let most = if allow_inset { 4 } else { 3 };
    if lengths.len() < 2 || lengths.len() > most {
        return Err(format!(
            "a shadow takes an x and y offset, then a blur{}",
            if allow_inset { " and a spread" } else { "" }
        ));
    }
    let blur = lengths.get(2).copied().unwrap_or(0.0);
    if blur < 0.0 {
        return Err("a shadow's blur cannot be negative".into());
    }
    // A shadow with no colour is the element's own, or black without one.
    let color = shade
        .or(units.color)
        .unwrap_or(blinc_core::Color::rgba(0.0, 0.0, 0.0, 1.0));
    Ok((
        inset,
        Shadow {
            offset_x: lengths[0] as f32,
            offset_y: lengths[1] as f32,
            blur: blur as f32,
            spread: lengths.get(3).copied().unwrap_or(0.0) as f32,
            color,
        },
    ))
}

/// `none`, or comma-separated layers, as outer and inset lists in CSS order.
pub fn parse(text: &str, units: &PaintUnits) -> Result<(Vec<Shadow>, Vec<Shadow>), String> {
    let (mut outer, mut inner) = (Vec::new(), Vec::new());
    if text.trim().eq_ignore_ascii_case("none") {
        return Ok((outer, inner));
    }
    for part in split(text, ',') {
        let (inset, shadow) = parse_one(&part, true, units)?;
        if inset {
            inner.push(shadow);
        } else {
            outer.push(shadow);
        }
    }
    Ok((outer, inner))
}
