//! Native feature tests and size-query expressions. Unknown size features keep
//! an unknown answer through negation, so `not (future-feature)` cannot match.
use super::{layout, paint, quantity, value};

/// Raw guards remain in compiled sheets so the loading native host decides
/// capabilities and container sizes. Every guard in a rule's span must hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Condition {
    Supports(super::Atom),
    Container(super::Atom),
}

fn negated(text: &str) -> bool {
    text.get(..3).is_some_and(|s| s.eq_ignore_ascii_case("not"))
        && text[3..].starts_with(char::is_whitespace)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expression {
    Feature(String),
    Not(Box<Self>),
    And(Vec<Self>),
    Or(Vec<Self>),
}

impl Expression {
    pub fn evaluate(&self, feature: &mut impl FnMut(&str) -> Option<bool>) -> Option<bool> {
        match self {
            Self::Feature(text) => feature(text),
            Self::Not(inner) => inner.evaluate(feature).map(|v| !v),
            Self::And(list) | Self::Or(list) => {
                let and = matches!(self, Self::And(_));
                let mut unknown = false;
                for item in list {
                    match item.evaluate(feature) {
                        Some(v) if v != and => return Some(v),
                        None => unknown = true,
                        _ => {}
                    }
                }
                (!unknown).then_some(and)
            }
        }
    }
}

/// Boolean grammar shared by supports and container conditions, with a bounded
/// nesting depth for runtime source and untrusted compiled sheets alike.
pub fn parse(text: &str) -> Result<Expression, String> {
    fn read(text: &str, depth: usize) -> Result<Expression, String> {
        let text = text.trim();
        if depth >= 64 || text.is_empty() {
            return Err("empty or excessively nested condition".into());
        }
        let mut level = 0i32;
        let mut quote = None;
        let mut escaped = false;
        let mut operators = Vec::new();
        let mut outer_end = None;
        for (at, ch) in text.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => level += 1,
                ')' => {
                    level -= 1;
                    if level < 0 {
                        return Err("unbalanced condition".into());
                    }
                    if level == 0 && outer_end.is_none() {
                        outer_end = Some(at);
                    }
                }
                _ if level == 0 && ch.is_ascii_whitespace() => {
                    let rest = text[at..].trim_start();
                    let start = text.len() - rest.len();
                    for op in ["and", "or"] {
                        if rest
                            .get(..op.len())
                            .is_some_and(|s| s.eq_ignore_ascii_case(op))
                            && rest[op.len()..].starts_with(char::is_whitespace)
                            && !operators.iter().any(|&(i, _)| i == start)
                        {
                            operators.push((start, op));
                        }
                    }
                }
                _ => {}
            }
        }
        if level != 0 || quote.is_some() || escaped {
            return Err("unbalanced condition".into());
        }
        if !operators.is_empty() {
            let op = operators[0].1;
            if operators.iter().any(|&(_, other)| other != op) {
                return Err("mixing and/or requires parentheses".into());
            }
            let mut list = Vec::new();
            let mut start = 0;
            for (at, _) in operators {
                let part = text[start..at].trim();
                if negated(part) {
                    return Err("a negated condition must be grouped before and/or".into());
                }
                list.push(read(part, depth + 1)?);
                start = at + op.len();
            }
            let part = text[start..].trim();
            if negated(part) {
                return Err("a negated condition must be grouped before and/or".into());
            }
            list.push(read(part, depth + 1)?);
            return Ok(if op == "and" {
                Expression::And(list)
            } else {
                Expression::Or(list)
            });
        }
        if negated(text) {
            let part = text[3..].trim();
            if negated(part) {
                return Err("nested negation requires parentheses".into());
            }
            return Ok(Expression::Not(Box::new(read(part, depth + 1)?)));
        }
        if text.starts_with('(') && outer_end == Some(text.len() - 1) {
            let inside = text[1..text.len() - 1].trim();
            if inside.starts_with('(') || negated(inside) {
                return read(inside, depth + 1);
            }
            if inside.is_empty() {
                return Err("empty feature".into());
            }
            return Ok(Expression::Feature(inside.into()));
        }
        // General-enclosed function syntax is a valid unknown feature.
        if outer_end == Some(text.len() - 1)
            && !text.contains([';', '{', '}'])
            && let Some((name, _)) = text.split_once('(')
            && !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Ok(Expression::Feature(text.into()));
        }
        Err("expected a condition in parentheses".into())
    }
    read(text, 0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContainerType {
    #[default]
    Normal,
    InlineSize,
    Size,
}
impl ContainerType {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(Self::Normal),
            "inline-size" => Some(Self::InlineSize),
            "size" => Some(Self::Size),
            _ => None,
        }
    }
}

pub fn names(text: &str) -> Option<Vec<String>> {
    if text.trim().eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let list: Vec<_> = text.split_whitespace().collect();
    if list.is_empty() || list.iter().any(|name| !identifier(name)) {
        return None;
    }
    Some(list.into_iter().map(str::to_string).collect())
}

fn identifier(name: &str) -> bool {
    let reserved = [
        "none",
        "and",
        "or",
        "not",
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ];
    !reserved.iter().any(|s| name.eq_ignore_ascii_case(s))
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '-')
        && name != "-"
        && !(name.starts_with('-') && name.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

pub fn container(text: &str) -> Option<(Vec<String>, ContainerType)> {
    match text.split_once('/') {
        Some((name, kind)) => Some((names(name)?, ContainerType::parse(kind)?)),
        None => Some((names(text)?, ContainerType::Normal)),
    }
}

/// Optional, case-sensitive name followed by a boolean size condition.
pub fn container_query(text: &str) -> Result<(Option<String>, Expression), String> {
    let text = text.trim();
    if text.starts_with('(')
        || negated(text)
        || text.starts_with("style(")
        || text.starts_with("scroll-state(")
    {
        return Ok((None, parse(text)?));
    }
    let (name, rest) = text
        .split_once(char::is_whitespace)
        .ok_or("expected a container condition")?;
    if !identifier(name) {
        return Err("invalid container name".into());
    }
    Ok((Some(name.into()), parse(rest)?))
}

/// Computed query-container metadata. Sizes are its content box, before pixel
/// rounding; no principal box has no size rather than a zero-sized box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Container {
    pub kind: ContainerType,
    pub names: Vec<String>,
    pub size: Option<(f64, f64)>,
    pub font_size: f64,
}

#[derive(Clone, Debug)]
pub struct SizeQuery {
    pub name: Option<String>,
    pub expression: Expression,
    /// Required physical axes: inline size is width in the supported horizontal writing mode.
    pub axes: u8,
}

fn axis(text: &str) -> u8 {
    match text.trim().to_ascii_lowercase().as_str() {
        "width" | "inline-size" => 1,
        "height" | "block-size" => 2,
        "aspect-ratio" | "orientation" => 3,
        _ => 0,
    }
}

/// A range split outside functions; values such as calc() keep their text intact.
fn range(text: &str) -> Option<(Vec<&str>, Vec<&str>)> {
    let (mut level, mut start, mut skip) = (0, 0, 0);
    let mut terms = Vec::new();
    let mut operators = Vec::new();
    for (at, ch) in text.char_indices() {
        if at < skip {
            continue;
        }
        match ch {
            '(' => level += 1,
            ')' => level -= 1,
            '<' | '>' | '=' if level == 0 => {
                let end = at
                    + if ch != '=' && text[at + 1..].starts_with('=') {
                        2
                    } else {
                        1
                    };
                terms.push(text[start..at].trim());
                operators.push(&text[at..end]);
                start = end;
                skip = end;
            }
            _ => {}
        }
    }
    if operators.is_empty() {
        return None;
    }
    terms.push(text[start..].trim());
    Some((terms, operators))
}

impl SizeQuery {
    pub fn parse(text: &str) -> Result<Self, String> {
        let (name, expression) = container_query(text)?;
        let mut axes = 0;
        // Walk every feature, including those in false/unknown boolean branches.
        fn collect(e: &Expression, axes: &mut u8) {
            match e {
                Expression::Feature(text) => {
                    if let Some((name, _)) = text.split_once(':') {
                        let name = name.trim().to_ascii_lowercase();
                        *axes |= axis(
                            name.strip_prefix("min-")
                                .or_else(|| name.strip_prefix("max-"))
                                .unwrap_or(&name),
                        );
                    } else if let Some((terms, _)) = range(text) {
                        for term in terms {
                            *axes |= axis(term);
                        }
                    } else {
                        *axes |= axis(text);
                    }
                }
                Expression::Not(e) => collect(e, axes),
                Expression::And(list) | Expression::Or(list) => {
                    for e in list {
                        collect(e, axes);
                    }
                }
            }
        }
        collect(&expression, &mut axes);
        Ok(Self {
            name,
            expression,
            axes,
        })
    }

    pub fn eligible(&self, container: &Container) -> bool {
        container.size.is_some()
            && (self
                .name
                .as_ref()
                .is_none_or(|name| container.names.contains(name)))
            && match container.kind {
                ContainerType::Normal => false,
                ContainerType::InlineSize => self.axes & 2 == 0,
                ContainerType::Size => true,
            }
    }

    pub fn evaluate(&self, container: &Container, units: &quantity::PaintUnits) -> Option<bool> {
        let (width, height) = container.size?;
        let units = quantity::PaintUnits {
            font_size: container.font_size,
            ..*units
        };
        let actual = |name: &str| match name.trim().to_ascii_lowercase().as_str() {
            "width" | "inline-size" => Some(width),
            "height" | "block-size" => Some(height),
            "aspect-ratio" if height != 0.0 => Some(width / height),
            _ => None,
        };
        let compare = |a: f64, op: &str, b: f64| match op {
            "=" => a == b,
            "<" => a < b,
            "<=" => a <= b,
            ">" => a > b,
            ">=" => a >= b,
            _ => false,
        };
        let value = |text: &str, ratio: bool| {
            if ratio {
                let (a, b) = text.split_once('/').unwrap_or((text, "1"));
                let a = a.trim().parse::<f64>().ok()?;
                let b = b.trim().parse::<f64>().ok()?;
                return (a.is_finite() && b.is_finite() && a >= 0.0 && b > 0.0).then_some(a / b);
            }
            if value::dimension(text).is_some_and(|(v, unit)| unit.is_empty() && v != 0.0) {
                return None;
            }
            let v = quantity::length(text, &units).ok()?;
            // Layout stores lengths as f32; compare at the same native precision.
            v.is_finite().then_some(v as f32 as f64)
        };
        self.expression.evaluate(&mut |text| {
            let text = text.to_ascii_lowercase();
            if let Some((name, expected)) = text.split_once(':') {
                let name = name.trim();
                if name == "orientation" {
                    return match expected.trim() {
                        "portrait" => Some(height >= width),
                        "landscape" => Some(width > height),
                        _ => None,
                    };
                }
                let (name, op) = if let Some(name) = name.strip_prefix("min-") {
                    (name, ">=")
                } else if let Some(name) = name.strip_prefix("max-") {
                    (name, "<=")
                } else {
                    (name, "=")
                };
                return Some(compare(
                    actual(name)?,
                    op,
                    value(expected.trim(), name == "aspect-ratio")?,
                ));
            }
            if let Some((terms, operators)) = range(&text) {
                let at = terms.iter().position(|name| actual(name).is_some())?;
                if terms.iter().filter(|name| axis(name) != 0).count() != 1
                    || !(operators.len() == 1
                        || operators.len() == 2
                            && at == 1
                            && operators[0].starts_with('<') == operators[1].starts_with('<')
                            && operators.iter().all(|op| *op != "="))
                {
                    return None;
                }
                let ratio = terms[at] == "aspect-ratio";
                let values = terms
                    .iter()
                    .map(|term| actual(term).or_else(|| value(term, ratio)))
                    .collect::<Option<Vec<_>>>()?;
                return Some(
                    operators
                        .iter()
                        .enumerate()
                        .all(|(i, op)| compare(values[i], op, values[i + 1])),
                );
            }
            if text == "orientation" {
                Some(true)
            } else {
                actual(&text).map(|v| v != 0.0)
            }
        })
    }
}

fn custom_property(name: &str) -> bool {
    name.starts_with("--")
        && name.len() > 2
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// Validate balanced declaration tokens and every var() argument, including
/// references inside calc() or a fallback. Quoted function text is inert.
fn declaration_tokens(text: &str) -> Option<bool> {
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut variable = false;
    for (at, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' => {
                if stack.len() >= 64 {
                    return None;
                }
                let start = text[..at]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| c.is_alphanumeric() || matches!(c, '-' | '_'))
                    .last()
                    .map_or(at, |(start, _)| start);
                stack.push((ch, at, ch == '(' && &text[start..at] == "var"));
            }
            ')' | ']' => {
                let (open, start, var) = stack.pop()?;
                if (open == '(') != (ch == ')') {
                    return None;
                }
                if var {
                    let name = value::split_ref(&text[start + 1..at], ',')[0];
                    if !custom_property(name) {
                        return None;
                    }
                    variable = true;
                }
            }
            ';' | '{' | '}' => return None,
            _ => {}
        }
    }
    (stack.is_empty() && quote.is_none() && !escaped).then_some(variable)
}

/// Test the property and its value, using the same strict native parsers as
/// layout/paint. Host-read text declarations have their actual supported grammar.
pub fn declaration_supported(text: &str) -> bool {
    let Some((name, raw)) = text.split_once(':') else {
        return false;
    };
    let name = name.trim().to_ascii_lowercase();
    let raw = raw.trim();
    let raw = super::parser::strip_important(raw).unwrap_or(raw);
    let Some(variable) = declaration_tokens(raw) else {
        return false;
    };
    if raw.is_empty() {
        return false;
    }
    if name.starts_with("--") {
        return custom_property(&name);
    }
    let host = matches!(
        name.as_str(),
        "font-size"
            | "font-weight"
            | "font-style"
            | "font-family"
            | "line-height"
            | "letter-spacing"
            | "text-align"
            | "white-space"
            | "text-transform"
            | "overflow-wrap"
            | "word-wrap"
            | "word-break"
            | "container-type"
            | "container-name"
            | "container"
    );
    let lays = layout::is_layout_property(&name);
    let paints = paint::is_paint_property(&name);
    if !host && !lays && !paints {
        return false;
    }
    if matches!(
        raw.to_ascii_lowercase().as_str(),
        "initial" | "inherit" | "unset"
    ) {
        return true;
    }
    // A syntactically valid var() is supported independent of its eventual value.
    if variable {
        return true;
    }
    let keyword = raw.to_ascii_lowercase();
    let choices = |list: &[&str]| list.contains(&keyword.as_str());
    if host {
        return match name.as_str() {
            "container-type" => ContainerType::parse(raw).is_some(),
            "container-name" => names(raw).is_some(),
            "container" => container(raw).is_some(),
            "font-family" => {
                super::font_face::families(raw).len() == value::split_ref(raw, ',').len()
            }
            "font-weight" => {
                choices(&["normal", "bold"])
                    || raw.parse::<u16>().is_ok_and(|n| (1..=1000).contains(&n))
            }
            "font-style" => choices(&["normal", "italic", "oblique"]),
            "font-size" | "letter-spacing" => {
                quantity::length(raw, &quantity::PaintUnits::default()).is_ok()
            }
            "line-height" => raw.parse::<f64>().is_ok_and(|v| v.is_finite() && v >= 0.0),
            "text-align" => choices(&["start", "end", "left", "right", "center", "justify"]),
            "white-space" => choices(&["normal", "nowrap", "pre", "pre-wrap", "pre-line"]),
            "text-transform" => choices(&["none", "uppercase", "lowercase", "capitalize"]),
            "overflow-wrap" | "word-wrap" => choices(&["normal", "break-word", "anywhere"]),
            "word-break" => choices(&["normal", "break-all", "keep-all", "break-word"]),
            _ => false,
        };
    }
    match name.as_str() {
        "background-size" => {
            return choices(&["auto", "auto auto", "cover", "contain", "100% 100%"]);
        }
        "background-repeat" => return choices(&["repeat", "repeat repeat"]),
        "background-position" => return choices(&["center", "center center", "50%", "50% 50%"]),
        _ => {}
    }
    (!lays
        || layout::layout_writes(&name, Some(raw), &layout::Units::default())
            .is_some_and(|r| r.is_ok()))
        && (!paints
            || paint::paint_writes(&name, Some(raw), &quantity::PaintUnits::default())
                .is_some_and(|r| r.is_ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feature_tests_validate_values_and_boolean_grouping() {
        let holds = |text| {
            parse(text)
                .unwrap()
                .evaluate(&mut |s| Some(declaration_supported(s)))
                == Some(true)
        };
        assert!(holds("(display: grid) and (width: calc(100% - 2rem))"));
        assert!(holds("not (display: invented-mode)"));
        assert!(holds(
            "((display: bogus) or (border-style: dotted)) and (text-align: center)"
        ));
        assert!(!holds("(unknown-property: 1)"));
        assert!(!holds("(background-size: fabricated)"));
        assert!(!holds("(width: calc(1px + 1s))"));
        assert!(parse("(display: grid) and (display: flex) or (color: red)").is_err());
        assert!(parse("not (display: grid) and (color: red)").is_err());
        assert!(parse("(display: grid) and not (color: red)").is_err());
        assert!(parse("not not (display: grid)").is_err());
        assert!(parse("not future(foo) junk(bar)").is_err());
        assert!(holds("not\t(display: invented-mode)"));
        assert!(holds("(display: grid ! IMPORTANT)"));
        assert!(holds("(width: calc(var(--size, 10px) + 2px))"));
        assert!(holds("(color: var(--colour, var(--fallback)))"));
        for text in [
            "(width: var(--size junk))",
            "(width: var(--size, var(invalid)))",
            "(width: var())",
        ] {
            assert!(!holds(text), "{text}");
        }
    }
    #[test]
    fn unknown_features_survive_negation() {
        assert_eq!(
            parse("not (future: foo)").unwrap().evaluate(&mut |_| None),
            None
        );
        assert!(
            container_query("Panel (width > 10em)")
                .unwrap()
                .0
                .as_deref()
                == Some("Panel")
        );
        assert!(container("sidebar / inline-size").is_some());
        assert!(container("sidebar / width").is_none());
        assert!(names("and").is_none());
        assert!(names("-1").is_none());
    }
    #[test]
    fn sizes_use_container_font_and_required_axes() {
        let c = Container {
            kind: ContainerType::InlineSize,
            names: vec!["Panel".into()],
            size: Some((320.0, 120.0)),
            font_size: 20.0,
        };
        let units = quantity::PaintUnits::default();
        for text in [
            "Panel (min-width: 16em)",
            "(200px < inline-size <= 320px)",
            "(width > 100px) and (not (width > 400px))",
            "not (width <= -100px)",
        ] {
            let q = SizeQuery::parse(text).unwrap();
            assert!(q.eligible(&c));
            assert_eq!(q.evaluate(&c, &units), Some(true), "{text}");
        }
        assert!(!SizeQuery::parse("(height > 1px)").unwrap().eligible(&c));
        assert!(
            !SizeQuery::parse("(MAX-HEIGHT: 200px)")
                .unwrap()
                .eligible(&c)
        );
        assert_eq!(
            SizeQuery::parse("not (future: foo)")
                .unwrap()
                .evaluate(&c, &units),
            None
        );
        assert_eq!(
            SizeQuery::parse("(width > 1px) or (future: foo)")
                .unwrap()
                .evaluate(&c, &units),
            Some(true)
        );
        assert_eq!(
            SizeQuery::parse("(width > 10px < 20px)")
                .unwrap()
                .evaluate(&c, &units),
            None
        );
    }
}
