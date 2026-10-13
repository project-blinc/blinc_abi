//! CSS colour gradients, as gradients over the unit
//! box, which painting scales to the node.

use super::color;
use super::quantity::{amount, angle};
use super::value::{call, dimension, split};
use blinc_core::Color;
use blinc_core::Point;
use blinc_core::layer::{Gradient, GradientSpace, GradientSpread, GradientStop};

/// One colour stop; `offset` is 0 to 1, or none to be spread between its neighbours.
#[derive(Clone, Copy, Debug)]
pub struct Stop {
    pub color: Color,
    pub offset: Option<f64>,
}

/// A gradient as written, before its stops are placed.
#[derive(Clone, Debug)]
pub enum CssGradient {
    /// Toward `angle` radians, 0 up and clockwise.
    Linear {
        angle: f64,
        stops: Vec<Stop>,
        repeat: bool,
    },
    /// A circle or ellipse about a point given as fractions of the box, reaching its farthest corner.
    Radial {
        circle: bool,
        repeat: bool,
        x: f64,
        y: f64,
        stops: Vec<Stop>,
    },
    /// Clockwise from the top, about a point in the box.
    Conic {
        angle: f64,
        x: f64,
        y: f64,
        stops: Vec<Stop>,
    },
}

/// Whether `text` is a gradient function.
pub fn is_gradient(text: &str) -> bool {
    call(text).is_some_and(|(name, _)| name.contains("gradient"))
}

pub fn parse(text: &str, current: Option<Color>) -> Result<CssGradient, String> {
    let (name, args) = call(text).ok_or_else(|| format!("expected a gradient, not \"{text}\""))?;
    let mut parts = split(&args, ',');
    match name.as_str() {
        "linear-gradient" | "repeating-linear-gradient" => {
            let mut turn = std::f64::consts::PI;
            let first = parts[0].to_ascii_lowercase();
            if let Some(side) = first.strip_prefix("to ") {
                turn = toward(side)?;
                parts.remove(0);
            } else if dimension(&first).is_some_and(|(_, unit)| unit != "%") {
                turn = angle(&first)?;
                parts.remove(0);
            }
            Ok(CssGradient::Linear {
                angle: turn,
                stops: stops(&parts, current, false)?,
                repeat: name.starts_with("repeating-"),
            })
        }
        "radial-gradient" | "repeating-radial-gradient" => {
            let (mut circle, mut x, mut y) = (false, 0.5, 0.5);
            let first = parts[0].to_ascii_lowercase();
            if !looks_like_stop(&first, current) {
                let words = split(&first, ' ');
                let at = words.iter().position(|w| w == "at");
                let shape = &words[..at.unwrap_or(words.len())];
                circle = shape.iter().any(|w| w == "circle");
                for w in shape {
                    if !matches!(w.as_str(), "circle" | "ellipse" | "farthest-corner") {
                        return Err(format!(
                            "radial-gradient() sizes other than farthest-corner are not supported: \"{w}\""
                        ));
                    }
                }
                if let Some(at) = at {
                    (x, y) = position(&words[at + 1..])?;
                }
                parts.remove(0);
            }
            Ok(CssGradient::Radial {
                circle,
                x,
                y,
                stops: stops(&parts, current, false)?,
                repeat: name.starts_with("repeating-"),
            })
        }
        "conic-gradient" | "repeating-conic-gradient" => {
            let (mut turn, mut x, mut y) = (0.0, 0.5, 0.5);
            if !looks_like_stop(&parts[0], current) {
                let words = split(&parts.remove(0), ' ');
                let mut i = 0;
                while i < words.len() {
                    match words[i].as_str() {
                        "from" if i + 1 < words.len() => {
                            turn = angle(&words[i + 1])?;
                            i += 2;
                        }
                        "at" => {
                            let end = words[i + 1..]
                                .iter()
                                .position(|w| w == "from")
                                .map_or(words.len(), |n| i + 1 + n);
                            if end == i + 1 {
                                return Err("conic-gradient() needs a position after at".into());
                            }
                            (x, y) = position(&words[i + 1..end])?;
                            i = end;
                        }
                        w => {
                            return Err(format!(
                                "expected from <angle> or at <position>, not \"{w}\""
                            ));
                        }
                    }
                }
            }
            let mut list = stops(&parts, current, true)?;
            if name.starts_with("repeating-") {
                // The core conic type has no spread field. Materialize periods
                // over one turn, retaining hard edges and avoiding unbounded allocation.
                let fixed = placed(&list);
                let first = fixed[0].1;
                let last = fixed[fixed.len() - 1].1;
                let period = last - first;
                if period <= 0.0 {
                    let color = average(
                        &fixed
                            .iter()
                            .map(|(color, offset)| GradientStop {
                                color: *color,
                                offset: *offset as f32,
                            })
                            .collect::<Vec<_>>(),
                    );
                    list = vec![
                        Stop {
                            color,
                            offset: Some(0.0),
                        },
                        Stop {
                            color,
                            offset: Some(1.0),
                        },
                    ];
                } else {
                    let start = ((0.0 - last) / period).floor();
                    let end = ((1.0 - first) / period).ceil();
                    if (end - start + 1.0) * fixed.len() as f64 > 4096.0 {
                        return Err("repeating-conic-gradient() exceeds 4096 expanded stops".into());
                    }
                    list = (start as i64..=end as i64)
                        .flat_map(|n| {
                            fixed.iter().map(move |(color, offset)| Stop {
                                color: *color,
                                offset: Some(offset + n as f64 * period),
                            })
                        })
                        .collect();
                }
            }
            Ok(CssGradient::Conic {
                angle: turn,
                x,
                y,
                stops: list,
            })
        }
        _ => Err(format!("{name}() is not a gradient")),
    }
}

/// `to right`, `to top left`, as an angle.
fn toward(side: &str) -> Result<f64, String> {
    let (mut dx, mut dy) = (0.0f64, 0.0f64);
    for word in split(side, ' ') {
        match word.as_str() {
            "top" => dy = -1.0,
            "bottom" => dy = 1.0,
            "left" => dx = -1.0,
            "right" => dx = 1.0,
            w => {
                return Err(format!(
                    "expected top, bottom, left or right after \"to\", not \"{w}\""
                ));
            }
        }
    }
    Ok(dx.atan2(-dy))
}

/// `center`, `left top`, `30% 70%`, as fractions of the box.
fn position(words: &[String]) -> Result<(f64, f64), String> {
    let (mut x, mut y) = (0.5, 0.5);
    for (i, w) in words.iter().enumerate() {
        match w.as_str() {
            "left" => x = 0.0,
            "right" => x = 1.0,
            "top" => y = 0.0,
            "bottom" => y = 1.0,
            "center" => {}
            _ => {
                let v = amount(w)?;
                if i == 0 {
                    x = v;
                } else {
                    y = v;
                }
            }
        }
    }
    Ok((x, y))
}

fn looks_like_stop(part: &str, current: Option<Color>) -> bool {
    split(part, ' ')
        .first()
        .is_some_and(|w| color::parse(w, current).is_ok())
}

fn stops(parts: &[String], current: Option<Color>, conic: bool) -> Result<Vec<Stop>, String> {
    if parts.len() < 2 {
        return Err("a gradient takes two colour stops or more".into());
    }
    let mut out = Vec::new();
    for part in parts {
        let words = split(part, ' ');
        let Some(first) = words.first() else {
            return Err("a gradient stop has no colour".into());
        };
        let color = color::parse(first, current)?;
        if words.len() == 1 {
            out.push(Stop {
                color,
                offset: None,
            });
        }
        if words.len() > 3 {
            return Err("a colour stop takes at most two positions".into());
        }
        for w in &words[1..] {
            out.push(Stop {
                color,
                offset: Some(if conic && !w.ends_with('%') {
                    angle(w)? / std::f64::consts::TAU
                } else {
                    amount(w)?
                }),
            });
        }
    }
    Ok(out)
}

/// Stops without an offset spread evenly between the ones around them, the
/// first at 0 and the last at 1; an offset before an earlier one moves up to it.
fn placed(stops: &[Stop]) -> Vec<(Color, f64)> {
    let mut offsets: Vec<Option<f64>> = stops.iter().map(|s| s.offset).collect();
    if let Some(first) = offsets.first_mut() {
        first.get_or_insert(0.0);
    }
    if let Some(last) = offsets.last_mut() {
        last.get_or_insert(1.0);
    }
    let mut previous = f64::NEG_INFINITY;
    for offset in offsets.iter_mut().flatten() {
        *offset = offset.max(previous);
        previous = *offset;
    }
    let mut i = 0;
    while i < offsets.len() {
        if offsets[i].is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let mut end = i;
        while offsets[end].is_none() {
            end += 1;
        }
        let (a, b) = (offsets[start].unwrap(), offsets[end].unwrap());
        for (k, offset) in offsets.iter_mut().enumerate().take(end).skip(i) {
            *offset = Some(a + (b - a) * (k - start) as f64 / (end - start) as f64);
        }
        i = end;
    }
    let mut out: Vec<(Color, f64)> = Vec::new();
    for (stop, offset) in stops.iter().zip(offsets) {
        let previous = out.last().map_or(f64::NEG_INFINITY, |(_, o)| *o);
        out.push((stop.color, offset.unwrap().max(previous)));
    }
    out
}

impl CssGradient {
    /// The gradient over the unit box. A linear one runs along its angle
    /// through the centre, reaching the corners as CSS's gradient line does.
    pub fn to_gradient(&self) -> Gradient {
        let stops = |list: &[Stop]| -> Vec<GradientStop> {
            placed(list)
                .into_iter()
                .map(|(color, offset)| GradientStop {
                    offset: offset as f32,
                    color,
                })
                .collect()
        };
        match self {
            CssGradient::Linear {
                angle,
                stops: list,
                repeat,
            } => {
                let (dx, dy) = (angle.sin(), -angle.cos());
                let half = (dx.abs() + dy.abs()) / 2.0;
                Gradient::Linear {
                    start: Point::new((0.5 - dx * half) as f32, (0.5 - dy * half) as f32),
                    end: Point::new((0.5 + dx * half) as f32, (0.5 + dy * half) as f32),
                    stops: stops(list),
                    space: GradientSpace::ObjectBoundingBox,
                    spread: if *repeat {
                        GradientSpread::Repeat
                    } else {
                        GradientSpread::Pad
                    },
                }
            }
            CssGradient::Radial {
                x,
                y,
                stops: list,
                repeat,
                ..
            } => {
                let far = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                    .iter()
                    .map(|(cx, cy)| ((cx - x).powi(2) + (cy - y).powi(2)).sqrt())
                    .fold(0.0, f64::max);
                Gradient::Radial {
                    center: Point::new(*x as f32, *y as f32),
                    radius: far as f32,
                    focal: None,
                    stops: stops(list),
                    space: GradientSpace::ObjectBoundingBox,
                    spread: if *repeat {
                        GradientSpread::Repeat
                    } else {
                        GradientSpread::Pad
                    },
                }
            }
            CssGradient::Conic {
                angle,
                x,
                y,
                stops: list,
            } => Gradient::Conic {
                center: Point::new(*x as f32, *y as f32),
                start_angle: *angle as f32,
                stops: stops(list),
                space: GradientSpace::ObjectBoundingBox,
            },
        }
    }
}

/// Average in premultiplied sRGB. A zero period spaces the stops evenly.
pub(crate) fn average(stops: &[GradientStop]) -> Color {
    if stops.len() < 2 {
        return stops.first().map_or(Color::TRANSPARENT, |s| s.color);
    }
    let length = stops.last().unwrap().offset - stops[0].offset;
    let mut rgba = [0.0; 4];
    for pair in stops.windows(2) {
        let weight = if length > 0.0 {
            (pair[1].offset - pair[0].offset) / length * 0.5
        } else {
            0.5 / (stops.len() - 1) as f32
        };
        for stop in pair {
            let c = stop.color;
            for (sum, channel) in rgba.iter_mut().zip([c.r * c.a, c.g * c.a, c.b * c.a, c.a]) {
                *sum += weight * channel;
            }
        }
    }
    if rgba[3] > 0.0 {
        Color::rgba(
            rgba[0] / rgba[3],
            rgba[1] / rgba[3],
            rgba[2] / rgba[3],
            rgba[3],
        )
    } else {
        Color::TRANSPARENT
    }
}
