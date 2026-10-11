//! Generated content, parsed once by the native CSS engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
}
impl PseudoElement {
    pub fn name(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Text(String),
    Image(String),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    pub content: Content,
    pub computed: super::cascade::Computed,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneratedNode {
    pub owner: u64,
    pub pseudo: PseudoElement,
    pub intrinsic: Option<(f32, f32)>,
}

pub fn parse(
    text: &str,
    attribute: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<Content>, String> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("normal") {
        return Ok(None);
    }
    let mut at = 0;
    let mut out = String::new();
    let mut terms = 0;
    while at < t.len() {
        while t.as_bytes().get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if at == t.len() {
            break;
        }
        if matches!(t.as_bytes()[at], b'\'' | b'"') {
            out.push_str(&quoted(t, &mut at)?);
        } else {
            let start = at;
            while t.as_bytes().get(at).is_some_and(u8::is_ascii_alphabetic) {
                at += 1;
            }
            let name = t[start..at].to_ascii_lowercase();
            if t.as_bytes().get(at) != Some(&b'(') {
                return Err("expected a string, attr(name) or url(source)".into());
            }
            at += 1;
            while t.as_bytes().get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            let argument = if matches!(t.as_bytes().get(at), Some(b'\'' | b'"')) {
                quoted(t, &mut at)?
            } else {
                let start = at;
                while t.as_bytes().get(at).is_some_and(|c| *c != b')') {
                    at += 1;
                }
                t[start..at].trim().to_string()
            };
            while t.as_bytes().get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            if t.as_bytes().get(at) != Some(&b')') {
                return Err("unclosed content function".into());
            }
            at += 1;
            match name.as_str() {
                "attr"
                    if !argument.is_empty()
                        && argument
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '-' || c == '_') =>
                {
                    out.push_str(&attribute(&argument).unwrap_or_default())
                }
                "url" if terms == 0 && t[at..].trim().is_empty() && !argument.is_empty() => {
                    return Ok(Some(Content::Image(argument)));
                }
                "url" => return Err("content accepts one non-empty url() by itself".into()),
                _ => {
                    return Err(
                        "expected a string, attr(name) or url(source); counters are not supported"
                            .into(),
                    );
                }
            }
        }
        terms += 1;
    }
    if terms == 0 {
        return Err("content is empty; use an empty quoted string or none".into());
    }
    Ok(Some(Content::Text(out)))
}

pub(crate) fn quoted(text: &str, at: &mut usize) -> Result<String, String> {
    let quote = text.as_bytes()[*at];
    *at += 1;
    let mut out = String::new();
    while *at < text.len() {
        let c = text[*at..].chars().next().unwrap();
        *at += c.len_utf8();
        if c as u32 == quote as u32 {
            return Ok(out);
        }
        match c {
            '\n' | '\r' | '\x0c' => return Err("newline in content string".into()),
            '\\' => {
                let Some(c) = text[*at..].chars().next() else {
                    return Err("unclosed content escape".into());
                };
                if c.is_ascii_hexdigit() {
                    let start = *at;
                    let mut count = 0;
                    while count < 6 && text.as_bytes().get(*at).is_some_and(u8::is_ascii_hexdigit) {
                        *at += 1;
                        count += 1;
                    }
                    let code = u32::from_str_radix(&text[start..*at], 16).unwrap();
                    out.push(
                        char::from_u32(code)
                            .filter(|c| *c != '\0')
                            .unwrap_or('\u{fffd}'),
                    );
                    if text
                        .as_bytes()
                        .get(*at)
                        .is_some_and(u8::is_ascii_whitespace)
                    {
                        let cr = text.as_bytes()[*at] == b'\r';
                        *at += 1;
                        if cr && text.as_bytes().get(*at) == Some(&b'\n') {
                            *at += 1;
                        }
                    }
                } else {
                    *at += c.len_utf8();
                    if c == '\r' && text.as_bytes().get(*at) == Some(&b'\n') {
                        *at += 1;
                    }
                    if !matches!(c, '\r' | '\n' | '\x0c') {
                        out.push(c);
                    }
                }
            }
            _ => out.push(c),
        }
    }
    Err("unclosed content string".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strings_attributes_and_images() {
        let attr = |name: &str| (name == "title").then(|| "world".to_string());
        assert_eq!(
            parse(r#""Hello " attr(title) "\21 ""#, &attr).unwrap(),
            Some(Content::Text("Hello world!".into()))
        );
        assert_eq!(
            parse(r#"'it\'s' "\1f600 ""#, &attr).unwrap(),
            Some(Content::Text("it's😀".into()))
        );
        assert_eq!(
            parse("attr(missing)", &attr).unwrap(),
            Some(Content::Text("".into()))
        );
        assert_eq!(
            parse("url('icon.svg')", &attr).unwrap(),
            Some(Content::Image("icon.svg".into()))
        );
        assert_eq!(parse("none", &attr).unwrap(), None);
        assert_eq!(parse("normal", &attr).unwrap(), None);
        for bad in [
            "",
            "counter(item)",
            "url()",
            "url(x) 'text'",
            "'unclosed",
            "attr(x",
            "attr(x y)",
        ] {
            assert!(parse(bad, &attr).is_err(), "{bad}");
        }
    }
}
