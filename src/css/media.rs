//! `@media` conditions: a query list holds when any of its queries does,
//! and a rule nested in several `@media` blocks needs every one of them.

use super::value;

/// A comparison in a media feature: `min-width` is `>=`, `max-width` `<=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaFeature {
    Width(Compare, f64),
    Height(Compare, f64),
    AspectRatio(Compare, f64),
    /// True for portrait.
    Orientation(bool),
    /// True for dark.
    ColorScheme(bool),
    /// A feature with a fixed answer here: `hover: hover` and `pointer: fine` hold, `print` does not.
    Fixed(bool),
    /// Both bounds of a range written with two comparisons, `400px <= width <= 800px`.
    Both(Box<MediaFeature>, Box<MediaFeature>),
}

/// One query of a list: an optional `not`, and features that must all hold.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaQuery {
    pub not: bool,
    pub features: Vec<MediaFeature>,
}

/// What a media query is asked about.
#[derive(Clone, Copy, Debug)]
pub struct MediaEnvironment {
    /// The viewport, in layout units.
    pub width: f64,
    pub height: f64,
    /// Whether the theme's scheme is dark, for `prefers-color-scheme`.
    pub dark: bool,
}

/// A query list, as written after `@media`; an error for one it cannot read.
pub fn parse(text: &str) -> Result<Vec<MediaQuery>, String> {
    value::split(text, ',')
        .into_iter()
        .map(|part| {
            if part.is_empty() {
                Err("an empty media query".to_string())
            } else {
                query(&part)
            }
        })
        .collect()
}

pub fn holds(list: &[MediaQuery], env: &MediaEnvironment) -> bool {
    list.iter()
        .any(|q| q.features.iter().all(|f| feature(f, env)) != q.not)
}

/// Whether every list holds: a rule nested in several `@media` blocks.
pub fn all_hold(all: Option<&[Vec<MediaQuery>]>, env: &MediaEnvironment) -> bool {
    all.is_none_or(|lists| lists.iter().all(|l| holds(l, env)))
}

fn query(text: &str) -> Result<MediaQuery, String> {
    let mut t = text.trim().to_lowercase();
    let mut not = false;
    if let Some(rest) = t.strip_prefix("not ") {
        not = true;
        t = rest.trim().to_string();
    } else if let Some(rest) = t.strip_prefix("only ") {
        t = rest.trim().to_string();
    }
    let mut features = Vec::new();
    // The media type, then features joined by "and".
    for (i, p) in split_and(&t).iter().enumerate() {
        if p.starts_with('(') {
            if !p.ends_with(')') {
                return Err(format!("expected a feature in parentheses, not \"{p}\""));
            }
            features.push(parse_feature(p[1..p.len() - 1].trim())?);
        } else if i == 0 {
            match p.as_str() {
                "all" | "screen" => {}
                "print" | "speech" => features.push(MediaFeature::Fixed(false)),
                _ => return Err(format!("unknown media type \"{p}\"")),
            }
        } else {
            return Err(format!(
                "expected a feature in parentheses after \"and\", not \"{p}\""
            ));
        }
    }
    Ok(MediaQuery { not, features })
}

/// `screen and (min-width: 600px) and (orientation: landscape)` at its top-level "and"s.
fn split_and(t: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut i) = (0i32, 0usize, 0usize);
    let b = t.as_bytes();
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ if depth == 0 && t[i..].starts_with(" and ") => {
                out.push(t[start..i].trim().to_string());
                start = i + 5;
                i += 5;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(t[start..].trim().to_string());
    out.retain(|s| !s.is_empty());
    out
}

/// A range feature's parts: `a`, its operator, `b`, and the second operator and `c` of `a op1 b op2 c`.
type Range = (String, String, String, Option<(String, String)>);

/// `a op b`, or `a op1 b op2 c`, split at its operators.
fn range(f: &str) -> Option<Range> {
    const OPS: [&str; 5] = ["<=", ">=", "<", ">", "="];
    let find = |s: &str, from: usize| -> Option<(usize, &'static str)> {
        let mut i = from;
        while i < s.len() {
            for op in OPS {
                if s[i..].starts_with(op) {
                    return Some((i, op));
                }
            }
            i += s[i..].chars().next().map_or(1, char::len_utf8);
        }
        None
    };
    // The first operator after at least one character, as `(.+?)\s*op` finds it.
    let (i, op1) = find(f, 1)?;
    let a = f[..i].trim().to_string();
    let rest = &f[i + op1.len()..];
    let rest_start = rest.len() - rest.trim_start().len();
    if a.is_empty() || rest.trim().is_empty() {
        return None;
    }
    match find(rest, rest_start + 1) {
        Some((j, op2)) if !rest[j + op2.len()..].trim().is_empty() => Some((
            a,
            op1.to_string(),
            rest[..j].trim().to_string(),
            Some((op2.to_string(), rest[j + op2.len()..].trim().to_string())),
        )),
        _ => Some((a, op1.to_string(), rest.trim().to_string(), None)),
    }
}

fn parse_feature(f: &str) -> Result<MediaFeature, String> {
    // Range syntax: "width >= 600px", "400px <= width <= 800px".
    if !f.contains(':')
        && let Some((a, op1, b, more)) = range(f)
    {
        if let Some((op2, c)) = more {
            // a op1 name op2 c: two features.
            let lower = ranged(&b, flip(compare(&op1)), &a)?;
            let upper = ranged(&b, compare(&op2), &c)?;
            return Ok(MediaFeature::Both(Box::new(lower), Box::new(upper)));
        }
        if is_name(&a) {
            return ranged(&a, compare(&op1), &b);
        }
        return ranged(&b, flip(compare(&op1)), &a);
    }
    let (name, value) = match f.find(':') {
        Some(i) => (f[..i].trim(), Some(f[i + 1..].trim())),
        None => (f.trim(), None),
    };
    Ok(match name {
        "width" | "min-width" | "max-width" | "height" | "min-height" | "max-height"
        | "aspect-ratio" | "min-aspect-ratio" | "max-aspect-ratio" => {
            let value = value.ok_or_else(|| format!("{name} needs a value"))?;
            let op = if name.starts_with("min-") {
                Compare::Ge
            } else if name.starts_with("max-") {
                Compare::Le
            } else {
                Compare::Eq
            };
            ranged(&name.replace("min-", "").replace("max-", ""), op, value)?
        }
        "orientation" => match value {
            Some("portrait") => MediaFeature::Orientation(true),
            Some("landscape") => MediaFeature::Orientation(false),
            v => {
                return Err(format!(
                    "orientation is portrait or landscape, not \"{}\"",
                    v.unwrap_or("null")
                ));
            }
        },
        "prefers-color-scheme" => match value {
            Some("dark") => MediaFeature::ColorScheme(true),
            Some("light") => MediaFeature::ColorScheme(false),
            v => {
                return Err(format!(
                    "prefers-color-scheme is light or dark, not \"{}\"",
                    v.unwrap_or("null")
                ));
            }
        },
        "hover" | "any-hover" => MediaFeature::Fixed(value.is_none_or(|v| v == "hover")),
        "pointer" | "any-pointer" => MediaFeature::Fixed(value.is_none_or(|v| v == "fine")),
        "prefers-reduced-motion" => MediaFeature::Fixed(value == Some("no-preference")),
        _ => return Err(format!("unknown media feature \"{name}\"")),
    })
}

fn is_name(s: &str) -> bool {
    matches!(s, "width" | "height" | "aspect-ratio")
}

fn ranged(name: &str, op: Compare, value: &str) -> Result<MediaFeature, String> {
    Ok(match name {
        "width" => MediaFeature::Width(op, length(value)?),
        "height" => MediaFeature::Height(op, length(value)?),
        "aspect-ratio" => MediaFeature::AspectRatio(op, ratio(value)?),
        _ => return Err(format!("unknown media feature \"{name}\"")),
    })
}

fn length(v: &str) -> Result<f64, String> {
    let t = v.trim().to_lowercase();
    if t == "auto" {
        return Err("\"auto\" is not allowed here".into());
    }
    let other = || format!("a media query length is in px, em or rem, not \"{v}\"");
    if value::call(&t)
        .is_some_and(|(name, _)| matches!(name.as_str(), "calc" | "min" | "max" | "clamp"))
    {
        return Err(other());
    }
    match value::dimension(&t) {
        None => Err(format!("expected a length, not \"{v}\"")),
        Some((x, u)) => match u.as_str() {
            "px" => Ok(x),
            "em" | "rem" => Ok(x * 16.0),
            "" if x == 0.0 => Ok(0.0),
            "" => Err(format!(
                "a length needs a unit, such as {}px",
                super::json::number(x)
            )),
            "%" | "vw" | "vh" | "vmin" | "vmax" => Err(other()),
            u => Err(format!("\"{u}\" is not a length unit this supports")),
        },
    }
}

fn ratio(v: &str) -> Result<f64, String> {
    let parts: Vec<&str> = v.split('/').collect();
    if parts.len() == 2 {
        Ok(value::number(parts[0])? / value::number(parts[1])?)
    } else {
        value::number(v)
    }
}

fn compare(op: &str) -> Compare {
    match op {
        "<" => Compare::Lt,
        "<=" => Compare::Le,
        ">" => Compare::Gt,
        ">=" => Compare::Ge,
        _ => Compare::Eq,
    }
}

fn flip(c: Compare) -> Compare {
    match c {
        Compare::Lt => Compare::Gt,
        Compare::Le => Compare::Ge,
        Compare::Gt => Compare::Lt,
        Compare::Ge => Compare::Le,
        Compare::Eq => Compare::Eq,
    }
}

fn feature(f: &MediaFeature, env: &MediaEnvironment) -> bool {
    let cmp = |op: Compare, a: f64, b: f64| match op {
        Compare::Eq => (a - b).abs() < 0.001,
        Compare::Lt => a < b,
        Compare::Le => a <= b,
        Compare::Gt => a > b,
        Compare::Ge => a >= b,
    };
    match f {
        MediaFeature::Width(op, px) => cmp(*op, env.width, *px),
        MediaFeature::Height(op, px) => cmp(*op, env.height, *px),
        MediaFeature::AspectRatio(op, r) => {
            env.height > 0.0 && cmp(*op, env.width / env.height, *r)
        }
        MediaFeature::Orientation(portrait) => (env.height >= env.width) == *portrait,
        MediaFeature::ColorScheme(dark) => env.dark == *dark,
        MediaFeature::Fixed(holds) => *holds,
        MediaFeature::Both(a, b) => feature(a, env) && feature(b, env),
    }
}
