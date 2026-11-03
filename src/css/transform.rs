//! CSS 2D transform functions, composed in order into one affine matrix.

use super::quantity::{PaintUnits, amount, angle, length};
use super::value::{call, number, split};
use blinc_core::layer::Affine2D;

pub const IDENTITY: Affine2D = Affine2D {
    elements: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
};

/// `transform-origin`: one or two of `left`, `center`, `right`, `top`, `bottom` and a percentage,
/// in either order when both are keywords, as percentages of the box from its top left. A length
/// needs the box's size, which a style does not have, so it is not read.
pub fn origin(text: &str) -> Result<[f32; 2], String> {
    #[derive(Clone, Copy, PartialEq)]
    enum Part {
        Horizontal(f32),
        Vertical(f32),
        Centre,
        Either(f32),
    }
    let part = |word: &str| -> Result<Part, String> {
        match word.to_ascii_lowercase().as_str() {
            "left" => Ok(Part::Horizontal(0.0)),
            "right" => Ok(Part::Horizontal(100.0)),
            "top" => Ok(Part::Vertical(0.0)),
            "bottom" => Ok(Part::Vertical(100.0)),
            "center" => Ok(Part::Centre),
            w => match w
                .strip_suffix('%')
                .and_then(|n| n.trim().parse::<f32>().ok())
            {
                Some(p) => Ok(Part::Either(p)),
                None => Err(format!(
                    "transform-origin reads keywords and percentages, not \"{word}\""
                )),
            },
        }
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    let parts = words
        .iter()
        .map(|w| part(w))
        .collect::<Result<Vec<_>, _>>()?;
    let (x, y) = match parts.as_slice() {
        [one] => match *one {
            Part::Vertical(y) => (50.0, y),
            Part::Horizontal(x) | Part::Either(x) => (x, 50.0),
            Part::Centre => (50.0, 50.0),
        },
        [a, b] => match (*a, *b) {
            (Part::Horizontal(x), Part::Vertical(y)) | (Part::Vertical(y), Part::Horizontal(x)) => {
                (x, y)
            }
            (Part::Either(x), Part::Either(y))
            | (Part::Either(x), Part::Vertical(y))
            | (Part::Horizontal(x), Part::Either(y)) => (x, y),
            (Part::Horizontal(x), Part::Centre)
            | (Part::Centre, Part::Horizontal(x))
            | (Part::Either(x), Part::Centre) => (x, 50.0),
            (Part::Vertical(y), Part::Centre)
            | (Part::Centre, Part::Vertical(y))
            | (Part::Centre, Part::Either(y)) => (50.0, y),
            (Part::Centre, Part::Centre) => (50.0, 50.0),
            _ => return Err("transform-origin names one position on each axis".into()),
        },
        _ => return Err("transform-origin takes one or two values".into()),
    };
    Ok([x, y])
}

/// `none`, or a list of `translate`, `scale`, `rotate`, `skew` and `matrix`
/// functions. The first is the outermost: `translate(10px, 0) scale(2)`
/// scales the box, then moves it.
pub fn parse(text: &str, units: &PaintUnits) -> Result<Affine2D, String> {
    if text.trim().eq_ignore_ascii_case("none") {
        return Ok(IDENTITY);
    }
    let mut m = [1.0f64, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut multiply = |n: [f64; 6]| {
        m = [
            m[0] * n[0] + m[2] * n[1],
            m[1] * n[0] + m[3] * n[1],
            m[0] * n[2] + m[2] * n[3],
            m[1] * n[2] + m[3] * n[3],
            m[0] * n[4] + m[2] * n[5] + m[4],
            m[1] * n[4] + m[3] * n[5] + m[5],
        ];
    };
    for part in split(text, ' ') {
        let (name, args) =
            call(&part).ok_or_else(|| format!("expected a transform function, not \"{part}\""))?;
        let args = split(&args, ',');
        let arity = |low: usize, high: usize| -> Result<(), String> {
            if args.len() < low || args.len() > high {
                let count = if low == high {
                    low.to_string()
                } else {
                    format!("{low} or {high}")
                };
                return Err(format!("{name}() takes {count} arguments"));
            }
            Ok(())
        };
        let px = |text: &str| -> Result<f64, String> {
            if text.trim().ends_with('%') {
                return Err("translate in % of the element's own size is not supported".into());
            }
            length(text, units)
        };
        match name.as_str() {
            "translate" => {
                arity(1, 2)?;
                let y = args.get(1).map_or(Ok(0.0), |a| px(a))?;
                multiply([1.0, 0.0, 0.0, 1.0, px(&args[0])?, y]);
            }
            "translatex" => {
                arity(1, 1)?;
                multiply([1.0, 0.0, 0.0, 1.0, px(&args[0])?, 0.0]);
            }
            "translatey" => {
                arity(1, 1)?;
                multiply([1.0, 0.0, 0.0, 1.0, 0.0, px(&args[0])?]);
            }
            "scale" => {
                arity(1, 2)?;
                let x = amount(&args[0])?;
                let y = args.get(1).map_or(Ok(x), |a| amount(a))?;
                multiply([x, 0.0, 0.0, y, 0.0, 0.0]);
            }
            "scalex" => {
                arity(1, 1)?;
                multiply([amount(&args[0])?, 0.0, 0.0, 1.0, 0.0, 0.0]);
            }
            "scaley" => {
                arity(1, 1)?;
                multiply([1.0, 0.0, 0.0, amount(&args[0])?, 0.0, 0.0]);
            }
            "rotate" | "rotatez" => {
                arity(1, 1)?;
                let r = angle(&args[0])?;
                multiply([r.cos(), r.sin(), -r.sin(), r.cos(), 0.0, 0.0]);
            }
            "skew" => {
                arity(1, 2)?;
                let y = args.get(1).map_or(Ok(0.0), |a| angle(a))?;
                multiply([1.0, y.tan(), angle(&args[0])?.tan(), 1.0, 0.0, 0.0]);
            }
            "skewx" => {
                arity(1, 1)?;
                multiply([1.0, 0.0, angle(&args[0])?.tan(), 1.0, 0.0, 0.0]);
            }
            "skewy" => {
                arity(1, 1)?;
                multiply([1.0, angle(&args[0])?.tan(), 0.0, 1.0, 0.0, 0.0]);
            }
            "matrix" => {
                arity(6, 6)?;
                let mut n = [0.0; 6];
                for (slot, a) in n.iter_mut().zip(&args) {
                    *slot = number(a)?;
                }
                multiply(n);
            }
            _ => return Err(format!("{name}() is not a 2D transform this supports")),
        }
    }
    Ok(Affine2D {
        elements: m.map(|v| v as f32),
    })
}
