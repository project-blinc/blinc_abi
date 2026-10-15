use super::cascade::*;
use super::*;

/// A tree for tests: elements by index, each with its parent and children.
#[derive(Default)]
struct Doc {
    parent: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    elements: Vec<Element>,
    containers: std::collections::HashMap<usize, conditional::Container>,
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
    fn container(&self, n: usize) -> Option<&conditional::Container> {
        self.containers.get(&n)
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
fn conditional_rules_keep_inactive_fallbacks_and_native_capabilities() {
    let source = "@keyframes move { to { opacity: 0.25 } } .x { width: 10px } @supports (display: grid) and (width: calc(100% - 1rem)) { .x { width: 20px } @font-face { font-family: Present; src: url(present.woff2); } } @supports (display: invented-mode) { .x { width: 30px } @font-face { font-family: Absent; src: url(absent.woff2); } @keyframes move { to { opacity: 1 } } } @supports not (made-up: 1) { .x { height: 40px } }";
    for compiled in [false, true] {
        let parsed = sheet(source);
        let mut c = Cascade::new();
        c.push(if compiled {
            compiled::decode(&compiled::encode(&parsed)).unwrap()
        } else {
            parsed
        });
        let mut doc = Doc::default();
        let element = el(&mut c, &["div"], &["x"]);
        let n = doc.add(None, element);
        let (computed, _) = c.style(&doc, n, None);
        assert_eq!(get(&c, &computed, "width").as_deref(), Some("20px"));
        assert_eq!(get(&c, &computed, "height").as_deref(), Some("40px"));
        let faces: Vec<_> = c.font_faces().map(|(_, _, s, f)| s.str(f.family)).collect();
        assert_eq!(faces, ["Present"]);
        let (s, frames) = c.keyframes("move").unwrap();
        assert_eq!(
            s.str(s.keyframe_declarations(&s.keyframe_list(frames)[0])[0].value),
            "0.25"
        );
    }
}

#[test]
fn container_guards_select_eligible_ancestors_and_revisit_false_reads() {
    let mut c = Cascade::new();
    c.push(sheet(".x { width: 10px } @container Panel (width >= 20em) { .x { width: 20px } } @container (height > 100px) { .x { height: 30px } }"));
    let mut doc = Doc::default();
    let root = doc.add(None, el(&mut c, &["div"], &[]));
    let parent = doc.add(Some(root), el(&mut c, &["div"], &[]));
    let n = doc.add(Some(parent), el(&mut c, &["div"], &["x"]));
    doc.containers.insert(
        root,
        conditional::Container {
            kind: conditional::ContainerType::Size,
            names: vec!["Panel".into()],
            size: Some((399.0, 200.0)),
            font_size: 20.0,
        },
    );
    doc.containers.insert(
        parent,
        conditional::Container {
            kind: conditional::ContainerType::InlineSize,
            names: Vec::new(),
            size: Some((900.0, 10.0)),
            font_size: 16.0,
        },
    );
    let (computed, deps) = c.style(&doc, n, None);
    assert_eq!(get(&c, &computed, "width").as_deref(), Some("10px"));
    assert_eq!(get(&c, &computed, "height").as_deref(), Some("30px"));
    assert!(!c.containers_changed(&doc, n, &deps.containers));
    doc.containers.get_mut(&root).unwrap().size = Some((400.0, 200.0));
    assert!(c.containers_changed(&doc, n, &deps.containers));
    let (computed, deps) = c.style(&doc, n, None);
    assert_eq!(get(&c, &computed, "width").as_deref(), Some("20px"));
    doc.containers.get_mut(&root).unwrap().names.clear();
    assert!(c.containers_changed(&doc, n, &deps.containers));
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
fn explain_says_where_each_declaration_came_from_and_which_wins() {
    let mut c = Cascade::new();
    let first = c.push(sheet(".a { color: red; width: 1px }\ndiv { color: blue }\n.a.b { color: green }\n.a { padding-top: 1px; height: 2px !important }"));
    let second = c.push(sheet(".b { padding: 4px }"));
    let mut doc = Doc::default();
    let mut e = el(&mut c, &["div"], &["a", "b"]);
    let w = c.intern("width");
    e.inline = vec![(w, "9px".into())];
    doc.add(None, e);
    let origins = c.explain(&doc, 0);
    let winner = |name: &str| origins.iter().find(|o| o.name == name && o.wins).cloned();
    let green = winner("color").unwrap();
    assert_eq!(
        (
            green.value.as_str(),
            green.selector.as_str(),
            green.line,
            green.sheet
        ),
        ("green", ".a.b", 3, Some(first))
    );
    // Overridden ones are listed, not winning: the type rule's and the first class rule's colour.
    assert_eq!(
        origins
            .iter()
            .filter(|o| o.name == "color" && !o.wins)
            .count(),
        2
    );
    // Its own declaration over the rules, with no sheet; an !important rule over it.
    let width = winner("width").unwrap();
    assert_eq!((width.value.as_str(), width.sheet), ("9px", None));
    assert!(winner("height").unwrap().important);
    // A later shorthand replaces an earlier longhand.
    assert!(winner("padding-top").is_none());
    assert_eq!(winner("padding").unwrap().sheet, Some(second));
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
fn background_shorthand_resets_mapping_but_stronger_longhands_survive() {
    let mut c = Cascade::new();
    c.push(sheet(".a { background-size: contain; background-position: right bottom; background-repeat: no-repeat; }
                  .a.b { background: url(tile.png); }
                  .a.b.c { background-repeat: repeat-x; }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["div"], &["a", "b", "c"]);
    doc.add(None, e);
    let s = &style_all(&c, &doc)[0];
    assert_eq!(get(&c, s, "background-size"), None);
    assert_eq!(get(&c, s, "background-position"), None);
    assert_eq!(get(&c, s, "background-repeat").as_deref(), Some("repeat-x"));
    assert_eq!(get(&c, s, "background").as_deref(), Some("url(tile.png)"));
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
fn a_value_outside_a_rule_resolves_as_an_element_would() {
    let mut c = Cascade::new();
    c.set_theme(&[("primary", "#0af")]);
    c.push(sheet(":root { --gap: 8px } .box { --ink: red }"));
    let mut doc = Doc::default();
    let b = el(&mut c, &["div"], &["box"]);
    let box_ = doc.add(None, b);
    let styles = style_all(&c, &doc);
    let values = &styles[box_].values;
    assert_eq!(c.resolve_vars("var(--ink) var(--gap)", values), "red 8px");
    assert_eq!(c.resolve_vars("var(--primary)", values), "#0af");
    assert_eq!(c.resolve_vars("var(--missing, 3px)", values), "3px");
    assert_eq!(c.resolve_vars("1px solid", &[]), "1px solid");
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

#[test]
fn font_math_is_computed_once_and_inherited_as_pixels() {
    let mut c = Cascade::new();
    c.set_environment(MediaEnvironment {
        width: 800.0,
        height: 600.0,
        dark: false,
    });
    c.push(sheet(".parent { font-size: 20px } .child { font-size: calc(150% + 2px); height: 1em } text.own { font-size: clamp(14px, 2vw, 20px) }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["div"], &["parent"]);
    let parent = doc.add(None, e);
    let e = el(&mut c, &["div"], &["child"]);
    let child = doc.add(Some(parent), e);
    let e = el(&mut c, &["text"], &[]);
    let inherited = doc.add(Some(child), e);
    let e = el(&mut c, &["text"], &["own"]);
    let own = doc.add(Some(child), e);
    let styles = style_all(&c, &doc);
    assert_eq!(styles[child].font_size, 32.0);
    assert_eq!(
        get(&c, &styles[child], "font-size").as_deref(),
        Some("32px")
    );
    assert_eq!(
        get(&c, &styles[inherited], "font-size").as_deref(),
        Some("32px")
    );
    assert_eq!(styles[own].font_size, 16.0);
    assert_eq!(get(&c, &styles[own], "font-size").as_deref(), Some("16px"));
    assert!(styles.iter().all(|s| s.errors.is_empty()));
    for bad in [
        "calc(1px / 0)",
        "calc(1e308px * 1e308)",
        "calc(-1px)",
        "calc(1px",
        "calc(1px) extra",
    ] {
        doc.elements[child].inline = vec![(c.atoms().find("font-size").unwrap(), bad.into())];
        let styles = style_all(&c, &doc);
        assert_eq!(styles[child].font_size, 20.0);
        assert!(styles[child].errors[0].contains("font-size"));
        assert!(styles[inherited].errors.is_empty());
        assert_eq!(styles[inherited].font_size, 20.0);
    }
}

#[test]
fn variables_leave_quoted_literals_and_quoted_fallback_parentheses_intact() {
    let mut cascade = Cascade::new();
    cascade.set_theme(&[("size", "20px"), ("label", "'Ready'")]);
    for literal in [
        "'var(--size)'",
        "\"var(--missing)\"",
        "'it\\'s var(--size)'",
        "url('var(--size)')",
    ] {
        assert_eq!(cascade.resolve_vars(literal, &[]), literal);
    }
    assert_eq!(
        cascade.resolve_vars("calc(var(--size) + 2px)", &[]),
        "calc(20px + 2px)"
    );
    assert_eq!(
        cascade.resolve_vars("var(--missing, 'a ) b')", &[]),
        "'a ) b'"
    );
    assert_eq!(cascade.resolve_vars("var(--label)", &[]), "'Ready'");
}

#[test]
fn inherited_cursor_and_list_markers_reach_non_text_hosts() {
    let mut c = Cascade::new();
    let first = c.push(sheet("ul { cursor: pointer; list-style-type: square } .own { cursor: default; list-style-type: lower-roman }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["ul"], &[]);
    let list = doc.add(None, e);
    let e = el(&mut c, &["li"], &[]);
    let item = doc.add(Some(list), e);
    let e = el(&mut c, &["li"], &["own"]);
    let own = doc.add(Some(list), e);
    let e = el(&mut c, &["div"], &[]);
    let child = doc.add(Some(own), e);
    let styles = style_all(&c, &doc);
    assert_eq!(get(&c, &styles[item], "cursor").as_deref(), Some("pointer"));
    assert_eq!(
        get(&c, &styles[item], "list-style-type").as_deref(),
        Some("square")
    );
    assert_eq!(
        get(&c, &styles[child], "cursor").as_deref(),
        Some("default")
    );
    assert_eq!(
        get(&c, &styles[child], "list-style-type").as_deref(),
        Some("lower-roman")
    );
    c.remove(first);
    let second = c.push(sheet("ul { cursor: wait; list-style-type: none }"));
    let styles = style_all(&c, &doc);
    assert_eq!(get(&c, &styles[item], "cursor").as_deref(), Some("wait"));
    assert_eq!(
        get(&c, &styles[item], "list-style-type").as_deref(),
        Some("none")
    );
    c.remove(second);
    let styles = style_all(&c, &doc);
    assert_eq!(get(&c, &styles[item], "cursor"), None);
    assert_eq!(get(&c, &styles[item], "list-style-type"), None);
}

#[test]
fn inherited_visibility_and_scrollbar_color_restyle_element_hosts() {
    let mut c = Cascade::new();
    let first = c.push(sheet("div { visibility: hidden; scrollbar-color: red blue; scrollbar-width: thin } .own { visibility: visible; scrollbar-color: auto }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["div"], &[]);
    let root = doc.add(None, e);
    let e = el(&mut c, &["section"], &[]);
    let child = doc.add(Some(root), e);
    let e = el(&mut c, &["section"], &["own"]);
    let own = doc.add(Some(root), e);
    let styles = style_all(&c, &doc);
    assert_eq!(
        get(&c, &styles[child], "visibility").as_deref(),
        Some("hidden")
    );
    assert_eq!(
        get(&c, &styles[child], "scrollbar-color").as_deref(),
        Some("red blue")
    );
    assert_eq!(get(&c, &styles[child], "scrollbar-width"), None);
    assert_eq!(
        get(&c, &styles[own], "visibility").as_deref(),
        Some("visible")
    );
    assert_eq!(
        get(&c, &styles[own], "scrollbar-color").as_deref(),
        Some("auto")
    );
    c.remove(first);
    let styles = style_all(&c, &doc);
    assert_eq!(get(&c, &styles[child], "visibility"), None);
    assert_eq!(get(&c, &styles[child], "scrollbar-color"), None);
}

#[test]
fn object_position_inherits_on_nontext_hosts_and_resets_after_sheet_removal() {
    let mut c = Cascade::new();
    let sheet = c.push(sheet("div { object-position: 12.5% 87.5%; object-fit: none } .own { object-position: right bottom }"));
    let mut doc = Doc::default();
    let e = el(&mut c, &["div"], &[]);
    let root = doc.add(None, e);
    let e = el(&mut c, &["img"], &[]);
    let child = doc.add(Some(root), e);
    let e = el(&mut c, &["img"], &["own"]);
    let own = doc.add(Some(root), e);
    let styles = style_all(&c, &doc);
    assert_eq!(
        get(&c, &styles[child], "object-position").as_deref(),
        Some("12.5% 87.5%")
    );
    assert_eq!(get(&c, &styles[child], "object-fit"), None);
    assert_eq!(
        get(&c, &styles[own], "object-position").as_deref(),
        Some("right bottom")
    );
    c.remove(sheet);
    assert_eq!(
        get(&c, &style_all(&c, &doc)[child], "object-position"),
        None
    );
}
