use super::*;

fn sheet(css: &str) -> Stylesheet {
    parse(css, None, &mut |_, _| None)
}

fn selectors(s: &Stylesheet, rule: usize) -> Vec<String> {
    s.rules[rule]
        .selectors
        .iter()
        .map(|x| x.to_string())
        .collect()
}

#[test]
fn rules_selectors_and_declarations() {
    let s = sheet(".card > .title:hover, #save { color: red; padding: 4px 8px !important }");
    assert!(s.diagnostics.is_empty(), "{}", s.report(None));
    assert_eq!(selectors(&s, 0), [".card > .title:hover", "#save"]);
    assert_eq!(s.rules[0].selectors[0].specificity(), 3000);
    assert_eq!(s.rules[0].selectors[1].specificity(), 1_000_000);
    let d = &s.rules[0].declarations;
    assert_eq!(
        (d[0].name.as_str(), d[0].value.as_str(), d[0].important),
        ("color", "red", false)
    );
    assert_eq!((d[1].value.as_str(), d[1].important), ("4px 8px", true));
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
    assert_eq!(
        s.rules[3].media.as_ref().unwrap()[0][0].features,
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
    assert_eq!(s.keyframes("spin").unwrap().frames[1].offsets, [0.5, 1.0]);
    assert_eq!(s.rules[1].declarations[0].value, "1px 2px");
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
    assert_eq!(s.imports, ["b.css"]);
    assert_eq!(selectors(&s, 0), [".b"]);
    assert!(s.rules[0].media.is_some());
}

#[test]
fn media_queries() {
    let env = media::MediaEnvironment {
        width: 700.0,
        height: 500.0,
        dark: true,
    };
    let q = media::parse("screen and (400px <= width <= 800px), print").unwrap();
    assert!(media::holds(&q, &env));
    assert!(!media::holds(
        &media::parse("(prefers-color-scheme: light)").unwrap(),
        &env
    ));
    assert!(media::parse("(frobs: 2)").is_err());
}

#[test]
fn compiled_bytes_give_the_same_sheet() {
    let s = sheet(
        ":root { --a: 1px } .x[y~=\"z\" i]:nth-last-of-type(-n+3) ~ p::placeholder { a: b !important }
         @media not print and (max-width: 40em), (orientation: portrait) { .q:where(.r) { c: d } }
         @keyframes k { 0% { o: 0 } to { o: 1 } } .err:nope { }",
    );
    let bytes = compiled::encode(&s);
    assert_eq!(compiled::decode(&bytes).unwrap(), s);
    assert!(compiled::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut other = bytes.clone();
    other[4] = 9;
    assert!(compiled::decode(&other).unwrap_err().contains("version"));
}
