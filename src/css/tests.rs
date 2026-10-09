use super::*;

fn sheet(css: &str) -> Stylesheet {
    parse(css, None, &mut |_, _| None)
}

fn selectors(s: &Stylesheet, rule: usize) -> Vec<String> {
    s.rule_selectors(&s.rules[rule])
        .iter()
        .map(|x| s.selector_text(x))
        .collect()
}

fn declaration(s: &Stylesheet, rule: usize, i: usize) -> (&str, &str, bool) {
    let d = &s.rule_declarations(&s.rules[rule])[i];
    (s.str(d.name), s.str(d.value), d.important)
}

#[test]
fn rules_selectors_and_declarations() {
    let s = sheet(".card > .title:hover, #save { color: red; padding: 4px 8px !important }");
    assert!(s.diagnostics.is_empty(), "{}", s.report(None));
    assert_eq!(selectors(&s, 0), [".card > .title:hover", "#save"]);
    let list = s.rule_selectors(&s.rules[0]);
    assert_eq!(
        (list[0].specificity, list[1].specificity),
        (3000, 1_000_000)
    );
    assert_eq!(declaration(&s, 0, 0), ("color", "red", false));
    assert_eq!(declaration(&s, 0, 1), ("padding", "4px 8px", true));
}

#[test]
fn equal_strings_are_one_atom() {
    let s = sheet(".a { color: red } .a:hover { color: red }");
    let first = s.rule_declarations(&s.rules[0])[0];
    let second = s.rule_declarations(&s.rules[1])[0];
    assert_eq!((first.name, first.value), (second.name, second.value));
    let class = |r: usize| s.compound_classes(s.subject(&s.rule_selectors(&s.rules[r])[0]))[0];
    assert_eq!(class(0), class(1));
    assert_eq!(s.atoms.find("red"), Some(first.value));
    assert_eq!(s.atoms.find("blue"), None);
}

#[test]
fn attributes_pseudos_and_nth() {
    let s = sheet(
        "button[data-variant=\"outline\"]:not(.x, :disabled) li:nth-child(2n+1):has(> img)::placeholder { a: b }",
    );
    assert!(s.diagnostics.is_empty(), "{}", s.report(None));
    assert_eq!(
        selectors(&s, 0),
        [
            "button[data-variant=\"outline\"]:not(.x, :disabled) li:nth-child(2n+1):has(> img)::placeholder"
        ]
    );
}

#[test]
fn nesting_flattens_and_keeps_source_order() {
    let s = sheet(
        ".a { color: red; &:hover { color: blue } .b { x: y } @media (min-width: 600px) { width: 1px } }",
    );
    assert!(s.diagnostics.is_empty(), "{}", s.report(None));
    assert_eq!(s.rules.len(), 4);
    assert_eq!(selectors(&s, 0), [".a"]);
    assert_eq!(selectors(&s, 1), [".a:hover"]);
    assert_eq!(selectors(&s, 2), [".a .b"]);
    assert_eq!(selectors(&s, 3), [".a"]);
    let list = s.rule_media(&s.rules[3])[0];
    assert_eq!(
        s.query_features(&s.list_queries(list)[0]),
        [MediaFeature::Width(Compare::Ge, 600.0)]
    );
}

#[test]
fn root_variables_keyframes_and_mixins() {
    let s = sheet(
        ":root { --gap: 4px; --gap: 8px }
         @keyframes spin { from { rotate: 0deg } 50%, to { rotate: 1turn } }
         @mixin pad($x, $y: 2px) { padding: $x $y; }
         .m { @include pad(1px); }",
    );
    assert!(s.diagnostics.is_empty(), "{}", s.report(None));
    assert_eq!(s.variable("gap"), Some("8px"));
    let spin = s.keyframes_named("spin").unwrap();
    assert_eq!(s.keyframe_offsets(&s.keyframe_list(spin)[1]), [0.5, 1.0]);
    assert_eq!(declaration(&s, 1, 0).1, "1px 2px");
}

#[test]
fn errors_skip_the_rule_and_name_its_place() {
    let s = sheet(".ok { a: b }\n.bad:frobnicate { a: b }\n.also-ok { c: d }\n@supports (x) { }");
    assert_eq!(s.rules.len(), 2);
    assert_eq!(s.diagnostics.len(), 2);
    assert_eq!((s.diagnostics[0].line, s.diagnostics[0].column), (2, 6));
    assert_eq!(
        s.diagnostics[0].message,
        ":frobnicate is not a pseudo-class this supports"
    );
    assert_eq!(s.diagnostics[1].severity, Severity::Warning);
}

#[test]
fn imports_read_through_the_loader_under_their_media() {
    let mut load = |path: &str, _: Option<&str>| {
        (path == "b.css").then(|| (".b { x: y }".to_string(), "b.css".to_string()))
    };
    let s = parse(
        "@import \"b.css\" print;\n.a { x: y }",
        Some("a.css"),
        &mut load,
    );
    assert!(s.diagnostics.is_empty(), "{}", s.report(None));
    assert_eq!(
        s.imports.iter().map(|&a| s.str(a)).collect::<Vec<_>>(),
        ["b.css"]
    );
    assert_eq!(selectors(&s, 0), [".b"]);
    assert!(s.rules[0].media.is_some());
}

#[test]
fn media_queries() {
    let env = MediaEnvironment {
        width: 700.0,
        height: 500.0,
        dark: true,
    };
    let s = sheet(
        "@media screen and (400px <= width <= 800px), print { .a { b: c } } @media (prefers-color-scheme: light) { .d { e: f } }",
    );
    assert!(s.media_holds(&s.rules[0], &env));
    assert!(!s.media_holds(&s.rules[1], &env));
    assert!(media::parse("(frobs: 2)").is_err());
}

#[test]
fn compiled_bytes_give_the_same_sheet() {
    let s = sheet(
        ":root { --a: 1px } .x[y~=\"z\" i]:nth-last-of-type(-n+3) ~ p::placeholder { a: b !important }
         @media not print and (max-width: 40em), (400px <= width <= 800px) { .q:where(.r):has(> .s:not(.t)) { c: d } }
         @keyframes k { 0% { o: 0 } to { o: 1 } } .err:nope { }",
    );
    let bytes = compiled::encode(&s);
    assert_eq!(compiled::decode(&bytes).unwrap(), s);
    assert_eq!(
        json::to_json(&compiled::decode(&bytes).unwrap()),
        json::to_json(&s)
    );
    assert!(compiled::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut other = bytes.clone();
    other[4] = 9;
    assert!(compiled::decode(&other).unwrap_err().contains("version"));
}

#[test]
fn damaged_bytes_are_refused_not_followed() {
    let s = sheet(".a:not(.b) { c: d } @media (1px <= width <= 2px) { .e { f: g } }");
    let bytes = compiled::encode(&s);
    // Every single-byte change either decodes to a sheet that checks out or is refused; none panics.
    for i in 6..bytes.len() {
        for v in [0u8, 1, 0x7f, 0xff] {
            let mut b = bytes.clone();
            b[i] = v;
            if let Ok(d) = compiled::decode(&b) {
                let _ = json::to_json(&d);
            }
        }
    }
}

#[test]
fn a_selector_list_on_its_own() {
    let (s, span) = parse_selectors(".card > .title, #save:not(.x)").unwrap();
    let text: Vec<String> = s
        .selector_list(span)
        .iter()
        .map(|x| s.selector_text(x))
        .collect();
    assert_eq!(text, [".card > .title", "#save:not(.x)"]);
    assert!(
        parse_selectors(".a:frob")
            .unwrap_err()
            .contains(":frob is not a pseudo-class")
    );
}
