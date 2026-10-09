//! A stylesheet as JSON, in one canonical shape: what the CLI prints with
//! `--json`, and what an SDK's own parser can be compared against.
//! Selectors are written as text, variables and keyframes sorted by name.

use super::*;

/// `v` as Haxe's `Std.string` writes a float: whole numbers without a point.
pub fn number(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn list<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    format!("[{}]", items.iter().map(f).collect::<Vec<_>>().join(","))
}

fn compare(c: Compare) -> &'static str {
    match c {
        Compare::Eq => "Eq",
        Compare::Lt => "Lt",
        Compare::Le => "Le",
        Compare::Gt => "Gt",
        Compare::Ge => "Ge",
    }
}

/// A media feature as `Width(Ge,600)`.
pub fn feature(f: &MediaFeature) -> String {
    match f {
        MediaFeature::Width(op, v) => format!("Width({},{})", compare(*op), number(*v)),
        MediaFeature::Height(op, v) => format!("Height({},{})", compare(*op), number(*v)),
        MediaFeature::AspectRatio(op, v) => format!("AspectRatio({},{})", compare(*op), number(*v)),
        MediaFeature::Orientation(b) => format!("Orientation({b})"),
        MediaFeature::ColorScheme(b) => format!("ColorScheme({b})"),
        MediaFeature::Fixed(b) => format!("Fixed({b})"),
        MediaFeature::Both(a, b) => format!("Both({},{})", feature(a), feature(b)),
    }
}

fn declaration(d: &Declaration) -> String {
    format!(
        "{{\"name\":{},\"value\":{},\"important\":{},\"line\":{},\"column\":{}}}",
        string(&d.name),
        string(&d.value),
        d.important,
        d.line,
        d.column
    )
}

pub fn to_json(sheet: &Stylesheet) -> String {
    let rules = list(&sheet.rules, |r| {
        let media = match &r.media {
            None => "null".to_string(),
            Some(lists) => list(lists, |l| {
                list(l, |q| {
                    format!(
                        "{{\"not\":{},\"features\":{}}}",
                        q.not,
                        list(&q.features, |f| string(&feature(f)))
                    )
                })
            }),
        };
        format!(
            "{{\"selectors\":{},\"specificity\":{},\"declarations\":{},\"media\":{},\"order\":{},\"line\":{}}}",
            list(&r.selectors, |s| string(&s.to_string())),
            list(&r.selectors, |s| s.specificity().to_string()),
            list(&r.declarations, declaration),
            media,
            r.order,
            r.line
        )
    });
    let mut variables = sheet.variables.clone();
    variables.sort_by(|a, b| a.0.cmp(&b.0));
    let mut keyframes: Vec<&Keyframes> = sheet.keyframes.iter().collect();
    keyframes.sort_by(|a, b| a.name.cmp(&b.name));
    format!(
        "{{\"rules\":{},\"variables\":{},\"keyframes\":{},\"imports\":{},\"diagnostics\":{}}}",
        rules,
        list(&variables, |(k, v)| format!(
            "[{},{}]",
            string(k),
            string(v)
        )),
        list(&keyframes, |k| format!(
            "{{\"name\":{},\"frames\":{}}}",
            string(&k.name),
            list(&k.frames, |f| format!(
                "{{\"offsets\":{},\"declarations\":{}}}",
                list(&f.offsets, |o| number(*o)),
                list(&f.declarations, declaration)
            ))
        )),
        list(&sheet.imports, |i| string(i)),
        list(&sheet.diagnostics, |d| format!(
            "{{\"severity\":\"{}\",\"message\":{},\"line\":{},\"column\":{},\"file\":{}}}",
            if d.severity == Severity::Error {
                "error"
            } else {
                "warning"
            },
            string(&d.message),
            d.line,
            d.column,
            d.file.as_deref().map_or("null".to_string(), string)
        ))
    )
}
