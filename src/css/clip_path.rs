//! The CSS `clip-path` shapes, in the element's own coordinates: `circle()`,
//! `ellipse()`, `inset()`, `rect()`, `xywh()`, `polygon()` and `path()`.

use super::quantity::{PaintUnits, length};
use super::svg_path;
use super::value::{call, dimension, split};
use blinc_core::{ClipLength, ClipPath};

/// A clip path and the rule that fills it.
#[derive(Clone, Debug)]
pub struct Clip {
    pub path: ClipPath,
    /// Whether a point is inside by the even-odd rule; the nonzero rule otherwise.
    pub even_odd: bool,
}

/// Between two rings of a path's vertices, a vertex at this x and y.
const BREAK: f32 = 1e30;

/// `none`, or one shape function.
pub fn parse(text: &str, units: &PaintUnits) -> Result<Option<Clip>, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let (name, body) =
        call(text).ok_or_else(|| format!("expected none or a shape function, not \"{text}\""))?;
    let shape = |path| {
        Ok(Some(Clip {
            path,
            even_odd: false,
        }))
    };
    match name.as_str() {
        "circle" => {
            let (size, at) = at_split(&body);
            if size.len() > 1 {
                return Err(format!(
                    "circle() takes one radius, not \"{}\"",
                    size.join(" ")
                ));
            }
            let radius = size
                .first()
                .map(|w| radius(w, units))
                .transpose()?
                .flatten();
            shape(ClipPath::Circle {
                radius,
                center: position(&at, units)?,
            })
        }
        "ellipse" => {
            let (size, at) = at_split(&body);
            if !size.is_empty() && size.len() != 2 {
                return Err(format!(
                    "ellipse() takes two radii or none, not \"{}\"",
                    size.join(" ")
                ));
            }
            let (rx, ry) = match size.as_slice() {
                [rx, ry] => (radius(rx, units)?, radius(ry, units)?),
                _ => (None, None),
            };
            shape(ClipPath::Ellipse {
                rx,
                ry,
                center: position(&at, units)?,
            })
        }
        "inset" => {
            let (words, round) = rounded(&body, units)?;
            let lengths = words
                .iter()
                .map(|w| len(w, units))
                .collect::<Result<Vec<_>, _>>()?;
            let [top, right, bottom, left] = match lengths.as_slice() {
                [a] => [*a, *a, *a, *a],
                [a, b] => [*a, *b, *a, *b],
                [a, b, c] => [*a, *b, *c, *b],
                [a, b, c, d] => [*a, *b, *c, *d],
                _ => {
                    return Err(format!(
                        "inset() takes one to four lengths, not \"{}\"",
                        words.join(" ")
                    ));
                }
            };
            shape(ClipPath::Inset {
                top,
                right,
                bottom,
                left,
                round,
            })
        }
        "rect" | "xywh" => {
            let (words, round) = rounded(&body, units)?;
            let lengths = words
                .iter()
                .map(|w| len(w, units))
                .collect::<Result<Vec<_>, _>>()?;
            let &[a, b, c, d] = lengths.as_slice() else {
                return Err(format!(
                    "{name}() takes four lengths, not \"{}\"",
                    words.join(" ")
                ));
            };
            shape(if name == "rect" {
                ClipPath::Rect {
                    top: a,
                    right: b,
                    bottom: c,
                    left: d,
                    round,
                }
            } else {
                ClipPath::Xywh {
                    x: a,
                    y: b,
                    w: c,
                    h: d,
                    round,
                }
            })
        }
        "polygon" => {
            let mut items = split(&body, ',');
            let even_odd = fill_rule(&mut items);
            if items.iter().all(String::is_empty) {
                return Err("polygon() needs points".into());
            }
            let points = items
                .iter()
                .map(|item| match split(item, ' ').as_slice() {
                    [x, y] => Ok((len(x, units)?, len(y, units)?)),
                    _ => Err(format!("a polygon point is two lengths, not \"{item}\"")),
                })
                .collect::<Result<_, String>>()?;
            Ok(Some(Clip {
                path: ClipPath::Polygon { points },
                even_odd,
            }))
        }
        "path" => {
            let mut items = split(&body, ',');
            let even_odd = if items.len() > 1 {
                let mut rule = vec![items.remove(0)];
                let even_odd = fill_rule(&mut rule);
                if !rule.is_empty() {
                    return Err(format!(
                        "path() takes a fill rule and a string, not \"{}\"",
                        body.trim()
                    ));
                }
                even_odd
            } else {
                false
            };
            let data = match items.as_slice() {
                [one] => unquote(one),
                _ => None,
            }
            .ok_or_else(|| {
                format!(
                    "path() takes its path data as a quoted string, not \"{}\"",
                    body.trim()
                )
            })?;
            let rings = svg_path::flatten(&svg_path::parse(data)?, 1.0);
            if rings.is_empty() {
                return Err("path() holds no shape".into());
            }
            let mut vertices = Vec::new();
            for ring in rings {
                if !vertices.is_empty() {
                    vertices.push((BREAK, BREAK));
                }
                vertices.extend(ring.into_iter().map(|(x, y)| (x as f32, y as f32)));
            }
            Ok(Some(Clip {
                path: ClipPath::Path { vertices },
                even_odd,
            }))
        }
        other => Err(format!(
            "clip-path {other}() is not circle, ellipse, inset, rect, xywh, polygon or path"
        )),
    }
}

/// A leading `nonzero` or `evenodd` taken off `items`; true for `evenodd`.
fn fill_rule(items: &mut Vec<String>) -> bool {
    match items.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("nonzero") => {
            items.remove(0);
            false
        }
        Some("evenodd") => {
            items.remove(0);
            true
        }
        _ => false,
    }
}

/// The text of a string in matching quotes.
fn unquote(text: &str) -> Option<&str> {
    let t = text.trim();
    let quote = t.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let inner = t.get(1..)?.strip_suffix(quote)?;
    (!inner.contains(quote)).then_some(inner)
}

/// The words before and after an `at`.
fn at_split(body: &str) -> (Vec<String>, Vec<String>) {
    let words = split(body, ' ');
    match words.iter().position(|w| w.eq_ignore_ascii_case("at")) {
        Some(i) => (words[..i].to_vec(), words[i + 1..].to_vec()),
        None => (words, Vec::new()),
    }
}

/// The words before a `round` and the one length after it, in pixels.
fn rounded(body: &str, units: &PaintUnits) -> Result<(Vec<String>, Option<f32>), String> {
    let words = split(body, ' ');
    let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case("round")) else {
        return Ok((words, None));
    };
    let radius = match &words[i + 1..] {
        [one] => match len(one, units)? {
            ClipLength::Px(v) if v >= 0.0 => v,
            _ => return Err(format!("round takes a length in pixels, not \"{one}\"")),
        },
        rest => {
            return Err(format!(
                "round takes one length in pixels, not \"{}\"",
                rest.join(" ")
            ));
        }
    };
    Ok((words[..i].to_vec(), Some(radius)))
}

/// A radius: a length, or `closest-side` for none.
fn radius(word: &str, units: &PaintUnits) -> Result<Option<ClipLength>, String> {
    match word.to_ascii_lowercase().as_str() {
        "closest-side" => Ok(None),
        "farthest-side" => {
            Err("farthest-side is not supported; give the radius or closest-side".into())
        }
        _ => match len(word, units)? {
            ClipLength::Px(v) | ClipLength::Percent(v) if v < 0.0 => {
                Err(format!("a radius is not negative, not \"{word}\""))
            }
            radius => Ok(Some(radius)),
        },
    }
}

/// After `at`: one or two positions, keywords or lengths; the centre when left out.
fn position(words: &[String], units: &PaintUnits) -> Result<(ClipLength, ClipLength), String> {
    let centre = ClipLength::Percent(50.0);
    let keyword = |word: &str, vertical: bool| match (word.to_ascii_lowercase().as_str(), vertical)
    {
        ("center", _) => Some(centre),
        ("left", false) | ("top", true) => Some(ClipLength::Percent(0.0)),
        ("right", false) | ("bottom", true) => Some(ClipLength::Percent(100.0)),
        _ => None,
    };
    let one = |word: &str, vertical: bool| match keyword(word, vertical) {
        Some(k) => Ok(k),
        None => len(word, units),
    };
    let is = |word: &str, names: &[&str]| names.iter().any(|n| word.eq_ignore_ascii_case(n));
    match words {
        [] => Ok((centre, centre)),
        // A lone vertical keyword sets y; anything else x.
        [w] if is(w, &["top", "bottom"]) => Ok((centre, one(w, true)?)),
        [w] => Ok((one(w, false)?, centre)),
        // Keywords come in either order: `top left` is `left top`.
        [a, b] if is(a, &["top", "bottom"]) || is(b, &["left", "right"]) => {
            Ok((one(b, false)?, one(a, true)?))
        }
        [a, b] => Ok((one(a, false)?, one(b, true)?)),
        _ => Err(format!(
            "a position is one or two values, not \"{}\"",
            words.join(" ")
        )),
    }
}

/// A length in pixels, or a percentage; a bare number is only 0.
fn len(word: &str, units: &PaintUnits) -> Result<ClipLength, String> {
    let (v, unit) = dimension(word)
        .ok_or_else(|| format!("expected a length or a percentage, not \"{word}\""))?;
    if unit == "%" {
        return Ok(ClipLength::Percent(v as f32));
    }
    if unit.is_empty() && v != 0.0 {
        return Err(format!("\"{word}\" needs a unit, px or %"));
    }
    length(word, units).map(|v| ClipLength::Px(v as f32))
}
