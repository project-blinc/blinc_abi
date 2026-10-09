use super::cascade::*;
use super::*;

/// A tree for tests: elements by index, each with its parent and children.
#[derive(Default)]
struct Doc {
    parent: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    elements: Vec<Element>,
}

impl Doc {
    fn add(&mut self, parent: Option<usize>, e: Element) -> usize {
        let n = self.elements.len();
        self.parent.push(parent);
        self.children.push(Vec::new());
        self.elements.push(e);
        if let Some(p) = parent {
            self.children[p].push(n);
        }
        n
    }
}

impl Tree for Doc {
    type Node = usize;
    fn parent(&self, n: usize) -> Option<usize> {
        self.parent[n]
    }
    fn children(&self, n: usize) -> Vec<usize> {
        self.children[n].clone()
    }
    fn element(&self, n: usize) -> Option<&Element> {
        self.elements.get(n)
    }
}

fn sheet(css: &str) -> Stylesheet {
    let s = parse(css, None, &mut |_, _| None);
    assert!(s.diagnostics.is_empty(), "{}", s.report(None));
    s
}

fn el(c: &mut Cascade, types: &[&str], classes: &[&str]) -> Element {
    Element {
        types: types.iter().map(|t| c.intern(t)).collect(),
        classes: classes.iter().map(|t| c.intern(t)).collect(),
        ..Default::default()
    }
}

fn get(c: &Cascade, computed: &Computed, name: &str) -> Option<String> {
    let a = c.atoms().find(name)?;
    computed
        .resolved
        .iter()
        .find(|(k, _)| *k == a)
        .map(|(_, v)| v.clone())
}

/// Every node's style, parents first.
fn style_all(c: &Cascade, doc: &Doc) -> Vec<Computed> {
    let mut out: Vec<Computed> = Vec::new();
    for n in 0..doc.elements.len() {
        let parent = doc.parent[n].map(|p| out[p].clone());
        out.push(c.style(doc, n, parent.as_ref()).0);
    }
    out
}

#[test]
fn specificity_then_order_then_important_and_inline() {
    let mut c = Cascade::new();
    c.push(sheet(".a { color: red; width: 1px } div { color: blue } .a.b { color: green } .a { height: 2px !important }"));
    c.push(sheet(".a { width: 3px }"));
    let mut doc = Doc::default();
    let mut e = el(&mut c, &["div"], &["a", "b"]);
    let (h, w) = (c.intern("height"), c.intern("width"));
    e.inline = vec![(h, "5px".into()), (w, "9px".into())];
    doc.add(None, e);
    let s = &style_all(&c, &doc)[0];
    assert_eq!(get(&c, s, "color").as_deref(), Some("green"));
    // Inline stands over every rule but an !important one.
    assert_eq!(get(&c, s, "width").as_deref(), Some("9px"));
    assert_eq!(get(&c, s, "height").as_deref(), Some("2px"));
}

#[test]
fn a_later_sheet_wins_a_tie() {
    let mut c = Cascade::new();
    c.push(sheet(".a { color: red }"));
    let later = c.push(sheet(".a { color: blue }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["div"], &["a"]);
    doc.add(None, e);
    assert_eq!(
        get(&c, &style_all(&c, &doc)[0], "color").as_deref(),
        Some("blue")
    );
    assert!(c.remove(later));
    assert_eq!(
        get(&c, &style_all(&c, &doc)[0], "color").as_deref(),
        Some("red")
    );
}

#[test]
fn a_shorthand_sets_its_longhands_anew() {
    let mut c = Cascade::new();
    c.push(sheet(".a { padding-top: 1px } .a.b { padding: 4px }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["div"], &["a", "b"]);
    doc.add(None, e);
    let s = &style_all(&c, &doc)[0];
    assert_eq!(get(&c, s, "padding-top"), None);
    assert_eq!(get(&c, s, "padding").as_deref(), Some("4px"));
}

#[test]
fn inheritance_reaches_text_and_variables_resolve() {
    let mut c = Cascade::new();
    c.set_theme(&[("primary", "#0af")]);
    c.push(sheet(
        ":root { --gap: 8px } .box { font-size: 20px; color: var(--ink, black); --ink: red; margin: var(--gap) }
         .inner { padding: var(--missing, 3px); background: var(--primary) } .em { font-size: 1.5em }",
    ));
    let mut doc = Doc::default();
    let b = el(&mut c, &["div"], &["box"]);
    let box_ = doc.add(None, b);
    let i = el(&mut c, &["div"], &["inner"]);
    let inner = doc.add(Some(box_), i);
    let t = el(&mut c, &["text"], &["em"]);
    let text = doc.add(Some(inner), t);
    let styles = style_all(&c, &doc);
    assert_eq!(get(&c, &styles[box_], "color").as_deref(), Some("red"));
    assert_eq!(get(&c, &styles[box_], "margin").as_deref(), Some("8px"));
    assert_eq!(get(&c, &styles[inner], "padding").as_deref(), Some("3px"));
    assert_eq!(
        get(&c, &styles[inner], "background").as_deref(),
        Some("#0af")
    );
    assert!(c.style(&doc, inner, Some(&styles[box_])).1.theme);
    // A plain element applies only its own declarations; a text element takes the inherited ones too.
    assert_eq!(get(&c, &styles[inner], "color"), None);
    assert_eq!(get(&c, &styles[text], "color").as_deref(), Some("red"));
    assert_eq!(styles[text].font_size, 30.0);
}

#[test]
fn combinators_structure_and_has() {
    let mut c = Cascade::new();
    c.push(sheet(
        "ul > li:first-child { a: first } li + li { b: next } li ~ .x { c: later } li:nth-child(2n+1) { d: odd }
         ul:has(> .x) { e: has } ul li:last-of-type { f: last } div li { g: no }",
    ));
    let mut doc = Doc::default();
    let u = el(&mut c, &["ul"], &[]);
    let ul = doc.add(None, u);
    let items: Vec<usize> = (0..3)
        .map(|i| {
            let e = el(&mut c, &["li"], if i == 2 { &["x"] } else { &[] });
            doc.add(Some(ul), e)
        })
        .collect();
    let s = style_all(&c, &doc);
    assert_eq!(get(&c, &s[items[0]], "a").as_deref(), Some("first"));
    assert_eq!(get(&c, &s[items[1]], "a"), None);
    assert_eq!(get(&c, &s[items[1]], "b").as_deref(), Some("next"));
    assert_eq!(get(&c, &s[items[2]], "c").as_deref(), Some("later"));
    assert_eq!(get(&c, &s[items[0]], "d").as_deref(), Some("odd"));
    assert_eq!(get(&c, &s[items[1]], "d"), None);
    assert_eq!(get(&c, &s[items[2]], "d").as_deref(), Some("odd"));
    assert_eq!(get(&c, &s[ul], "e").as_deref(), Some("has"));
    assert_eq!(get(&c, &s[items[2]], "f").as_deref(), Some("last"));
    assert_eq!(get(&c, &s[items[0]], "g"), None);
}

#[test]
fn states_attributes_and_dependencies() {
    let mut c = Cascade::new();
    c.push(sheet(".b:hover { a: hover } .b:enabled { e: on } [data-v^=\"out\"] { o: yes } .p:focus-within .b { f: within }"));
    let mut doc = Doc::default();
    let mut p = el(&mut c, &["div"], &["p"]);
    p.states = States(States::bit("focus-within").unwrap());
    let parent = doc.add(None, p);
    let mut b = el(&mut c, &["button"], &["b"]);
    let (dv, out) = (c.intern("data-v"), c.intern("outline"));
    b.attributes = vec![(dv, out)];
    b.states = States(States::bit("hover").unwrap());
    let button = doc.add(Some(parent), b);
    let s = style_all(&c, &doc);
    let (st, deps) = c.style(&doc, button, Some(&s[parent]));
    assert_eq!(get(&c, &st, "a").as_deref(), Some("hover"));
    assert_eq!(get(&c, &st, "e").as_deref(), Some("on"));
    assert_eq!(get(&c, &st, "o").as_deref(), Some("yes"));
    assert_eq!(get(&c, &st, "f").as_deref(), Some("within"));
    assert!(
        deps.states
            .contains(&(button, States::bit("hover").unwrap()))
    );
    assert!(
        deps.states
            .contains(&(parent, States::bit("focus-within").unwrap()))
    );
}

#[test]
fn media_queries_follow_the_environment() {
    let mut c = Cascade::new();
    c.push(sheet(".a { w: narrow } @media (min-width: 600px) { .a { w: wide } } @media (prefers-color-scheme: dark) { .a { s: dark } }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["div"], &["a"]);
    doc.add(None, e);
    c.set_environment(MediaEnvironment {
        width: 400.0,
        height: 300.0,
        dark: false,
    });
    let s = style_all(&c, &doc);
    assert_eq!(
        (get(&c, &s[0], "w").as_deref(), get(&c, &s[0], "s")),
        (Some("narrow"), None)
    );
    c.set_environment(MediaEnvironment {
        width: 800.0,
        height: 300.0,
        dark: true,
    });
    let s = style_all(&c, &doc);
    assert_eq!(
        (
            get(&c, &s[0], "w").as_deref(),
            get(&c, &s[0], "s").as_deref()
        ),
        (Some("wide"), Some("dark"))
    );
}
