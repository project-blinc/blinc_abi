//! Font descriptors and CSS matching, shared by adapters loading bundled faces.
use super::{Atom, Span, Stylesheet, generated, tree::Declaration, value};

#[derive(Clone, Debug, PartialEq)]
pub struct FontSource {
    pub name: Atom,
    pub local: bool,
    pub format: Option<Atom>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontFace {
    pub family: Atom,
    pub sources: Vec<FontSource>,
    pub weight: [u16; 2],
    pub italic: bool,
    pub media: Option<Span>,
    /// The stylesheet defining relative URLs, including imported sheets.
    pub file: Option<Atom>,
    pub line: u32,
    pub column: u32,
}

fn string(text: &str) -> Result<String, String> {
    let t = text.trim();
    if matches!(t.as_bytes().first(), Some(b'\'' | b'"')) {
        let mut at = 0;
        let out = generated::quoted(t, &mut at)?;
        if at != t.len() {
            return Err("unexpected text after a font string".into());
        }
        Ok(out)
    } else if !t.is_empty() && !t.contains([',', '(', ')', '\'', '"']) {
        Ok(t.to_string())
    } else {
        Err("expected a non-empty font name or URL".into())
    }
}

fn weights(text: &str) -> Result<[u16; 2], String> {
    let values = value::split_ref(text, ' ');
    let weight = |v: &str| match v.to_ascii_lowercase().as_str() {
        "normal" => Ok(400),
        "bold" => Ok(700),
        _ => v
            .parse::<u16>()
            .ok()
            .filter(|w| (1..=1000).contains(w))
            .ok_or("font-weight must be 1..1000".to_string()),
    };
    let range = match values.as_slice() {
        [a] => [weight(a)?, weight(a)?],
        [a, b] => [weight(a)?, weight(b)?],
        _ => return Err("expected one font weight or an ascending range".into()),
    };
    if range[0] > range[1] {
        return Err("font-weight range must ascend".into());
    }
    Ok(range)
}

pub(crate) fn parse(
    sheet: &mut Stylesheet,
    declarations: &[Declaration],
) -> Result<FontFace, String> {
    let get = |name: &str| {
        declarations
            .iter()
            .rev()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .map(|d| d.value.as_str())
    };
    if declarations.iter().any(|d| d.important) {
        return Err("font descriptors do not accept !important".into());
    }
    let family = string(get("font-family").ok_or("font-family is required")?)?;
    if family.is_empty()
        || matches!(
            family.to_ascii_lowercase().as_str(),
            "initial"
                | "inherit"
                | "unset"
                | "default"
                | "serif"
                | "sans-serif"
                | "monospace"
                | "system-ui"
        )
    {
        return Err("font-family must name one custom family".into());
    }
    let weight = weights(get("font-weight").unwrap_or("normal"))?;
    let italic = match get("font-style")
        .unwrap_or("normal")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "normal" => false,
        "italic" | "oblique" => true,
        _ => return Err("font-style supports normal, italic or oblique".into()),
    };
    if get("unicode-range").is_some()
        || get("font-stretch").is_some_and(|v| !v.eq_ignore_ascii_case("normal") && v != "100%")
    {
        return Err("unicode-range and stretched faces are not supported".into());
    }
    let mut sources = Vec::new();
    for src in value::split_ref(get("src").ok_or("src is required")?, ',') {
        let terms = value::split_ref(src, ' ');
        let Some((kind, argument)) = terms.first().and_then(|t| value::call(t)) else {
            return Err("src expects url() or local(), optionally followed by format()".into());
        };
        let local = match kind.as_str() {
            "local" => true,
            "url" => false,
            _ => return Err("src expects url() or local()".into()),
        };
        let name = string(&argument)?;
        if name.is_empty() {
            return Err("empty font source".into());
        }
        let format = match terms.as_slice() {
            [_] => None,
            [_, format] if !local => {
                let Some((kind, arg)) = value::call(format) else {
                    return Err("expected format()".into());
                };
                if kind != "format" {
                    return Err("expected format()".into());
                }
                Some(sheet.atoms.intern(&string(&arg)?.to_ascii_lowercase()))
            }
            _ => return Err("unsupported font source qualifiers".into()),
        };
        sources.push(FontSource {
            name: sheet.atoms.intern(&name),
            local,
            format,
        });
    }
    Ok(FontFace {
        family: sheet.atoms.intern(&family),
        sources,
        weight,
        italic,
        media: None,
        file: None,
        line: 0,
        column: 0,
    })
}

/// CSS Fonts weight search order, after preferring the requested italic style.
/// A containing range wins; for equal ranks a later declaration wins.
pub fn match_face(faces: &[([u16; 2], bool)], weight: u16, italic: bool) -> Option<usize> {
    let rank = |range: [u16; 2]| {
        let w = weight.clamp(range[0], range[1]);
        if w == weight {
            return (0u8, 0u16);
        }
        if (400..=500).contains(&weight) {
            if w >= weight && w <= 500 {
                (1, w - weight)
            } else if w < weight {
                (2, weight - w)
            } else {
                (3, w - weight)
            }
        } else if weight < 400 {
            if w < weight {
                (1, weight - w)
            } else {
                (2, w - weight)
            }
        } else if w > weight {
            (1, w - weight)
        } else {
            (2, weight - w)
        }
    };
    faces
        .iter()
        .enumerate()
        .min_by_key(|(i, (range, style))| (*style != italic, rank(*range), std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_matching_follows_css_search_and_source_order() {
        let faces = [
            ([300, 300], false),
            ([500, 500], false),
            ([700, 700], false),
            ([400, 400], true),
        ];
        assert_eq!(match_face(&faces, 400, false), Some(1));
        assert_eq!(match_face(&faces, 350, false), Some(0));
        assert_eq!(match_face(&faces, 600, false), Some(2));
        assert_eq!(match_face(&faces, 700, true), Some(3));
        assert_eq!(
            match_face(&[([100, 900], false), ([400, 600], false)], 450, false),
            Some(1)
        );
    }
    #[test]
    fn runtime_compiled_imported_faces_keep_sources_and_conditions() {
        let css = "@import 'fonts/faces.css'; @font-face { font-family: Example; src: url(a.ttf); } .ok { color: red; }";
        let sheet = super::super::parse(css, Some("/app/main.css"), &mut |_, _| {
            Some(("@media (min-width: 300px) { @font-face { font-family: 'Example'; src: local('Fira Code'), url(regular.woff2) format('woff2'); font-weight: 100 900; font-style: italic; } }".into(), "/app/fonts/faces.css".into()))
        });
        assert!(sheet.diagnostics.is_empty(), "{:?}", sheet.diagnostics);
        assert_eq!(sheet.font_faces.len(), 2);
        let face = &sheet.font_faces[0];
        assert_eq!(sheet.str(face.family), "Example");
        assert_eq!(face.weight, [100, 900]);
        assert!(face.italic);
        assert!(face.sources[0].local);
        assert_eq!(sheet.str(face.sources[1].name), "regular.woff2");
        assert_eq!(sheet.str(face.file.unwrap()), "/app/fonts/faces.css");
        assert!(!sheet.media_span_holds(
            face.media,
            &super::super::MediaEnvironment {
                width: 200.,
                height: 100.,
                dark: false
            }
        ));
        let bytes = super::super::compiled::encode(&sheet);
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 3);
        assert_eq!(super::super::compiled::decode(&bytes).unwrap(), sheet);
        for end in 0..bytes.len() {
            assert!(super::super::compiled::decode(&bytes[..end]).is_err());
        }
        let old = super::super::parse(".ok { color: red; }", None, &mut |_, _| None);
        let old_bytes = super::super::compiled::encode(&old);
        assert_eq!(u16::from_le_bytes([old_bytes[4], old_bytes[5]]), 2);
        assert_eq!(super::super::compiled::decode(&old_bytes).unwrap(), old);
    }
    #[test]
    fn bad_face_does_not_discard_following_rules() {
        let sheet = super::super::parse(
            "@font-face { font-family: A; src: url(); } @font-face { font-family: B; src: url(b.ttf); font-weight: 900 100; } .ok { color: red; }",
            None,
            &mut |_, _| None,
        );
        assert!(sheet.font_faces.is_empty());
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.diagnostics.len(), 2);
    }
}
