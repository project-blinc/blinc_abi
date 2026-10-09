//! A stylesheet as JSON, in one canonical shape: what the CLI prints with
//! `--json`, and what an SDK's own parser can be compared against.
//! Selectors are written as text, variables and keyframes sorted by name.

use super::*;

/// `v` with whole numbers written without a point.
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
pub fn feature(sheet: &Stylesheet, f: &MediaFeature) -> String {
    match *f {
        MediaFeature::Width(op, v) => format!("Width({},{})", compare(op), number(v)),
        MediaFeature::Height(op, v) => format!("Height({},{})", compare(op), number(v)),
        MediaFeature::AspectRatio(op, v) => format!("AspectRatio({},{})", compare(op), number(v)),
        MediaFeature::Orientation(b) => format!("Orientation({b})"),
        MediaFeature::ColorScheme(b) => format!("ColorScheme({b})"),
        MediaFeature::Fixed(b) => format!("Fixed({b})"),
        MediaFeature::Both(a, b) => format!(
            "Both({},{})",
            feature(sheet, &sheet.features[a as usize]),
            feature(sheet, &sheet.features[b as usize])
        ),
    }
}

fn declaration(sheet: &Stylesheet, d: &Declaration) -> String {
    format!(
        "{{\"name\":{},\"value\":{},\"important\":{},\"line\":{},\"column\":{}}}",
        string(sheet.str(d.name)),
        string(sheet.str(d.value)),
        d.important,
        d.line,
        d.column
    )
}

pub fn to_json(sheet: &Stylesheet) -> String {
    let rules = list(&sheet.rules, |r| {
        let media = match r.media {
            None => "null".to_string(),
            Some(_) => list(sheet.rule_media(r), |&l| {
                list(sheet.list_queries(l), |q| {
                    format!(
                        "{{\"not\":{},\"features\":{}}}",
                        q.not,
                        list(sheet.query_features(q), |f| string(&feature(sheet, f)))
                    )
                })
            }),
        };
        format!(
            "{{\"selectors\":{},\"specificity\":{},\"declarations\":{},\"media\":{},\"order\":{},\"line\":{}}}",
            list(sheet.rule_selectors(r), |s| string(&sheet.selector_text(s))),
            list(sheet.rule_selectors(r), |s| s.specificity.to_string()),
            list(sheet.rule_declarations(r), |d| declaration(sheet, d)),
            media,
            r.order,
            r.line
        )
    });
    let mut variables: Vec<(&str, &str)> = sheet
        .variables
        .iter()
        .map(|&(k, v)| (sheet.str(k), sheet.str(v)))
        .collect();
    variables.sort_by(|a, b| a.0.cmp(b.0));
    let mut keyframes: Vec<&Keyframes> = sheet.keyframes.iter().collect();
    keyframes.sort_by(|a, b| sheet.str(a.name).cmp(sheet.str(b.name)));
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
            string(sheet.str(k.name)),
            list(sheet.keyframe_list(k), |f| format!(
                "{{\"offsets\":{},\"declarations\":{}}}",
                list(sheet.keyframe_offsets(f), |o| number(*o)),
                list(sheet.keyframe_declarations(f), |d| declaration(sheet, d))
            ))
        )),
        list(&sheet.imports, |&i| string(sheet.str(i))),
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
