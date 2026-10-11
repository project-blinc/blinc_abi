use super::cascade::{Element, States};
use super::styled::Styles;
use super::{MediaEnvironment, parse};
use crate::context::{LayoutContext, Node};

fn element(s: &mut Styles, types: &[&str], classes: &[&str]) -> Element {
    Element {
        types: types.iter().map(|t| s.intern(t)).collect(),
        classes: classes.iter().map(|c| s.intern(c)).collect(),
        ..Default::default()
    }
}

fn bounds(ctx: &LayoutContext, n: Node) -> [f32; 4] {
    let mut out = [0.0; 4];
    ctx.read_bounds(&[n], &mut out).unwrap();
    out
}

#[test]
fn a_sheet_lays_a_context_out_and_follows_states_and_classes() {
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let items: Vec<Node> = (0..2)
        .map(|_| ctx.create_node(Default::default()).unwrap())
        .collect();
    for &i in &items {
        ctx.insert_before(root, i, None).unwrap();
    }
    let mut s = Styles::new();
    s.set_environment(MediaEnvironment {
        width: 800.0,
        height: 600.0,
        dark: false,
    });
    let sheet = parse(
        ".row { display: flex; flex-direction: row; width: 200px; height: 50px; padding: 10px }
         .item { width: 50%; height: 20px; color: var(--ink, red) } .item:hover { height: 30px } .tall { height: 2em; font-size: 10px }",
        None,
        &mut |_, _| None,
    );
    s.cascade_mut().push(sheet);
    let row = element(&mut s, &["div"], &["row"]);
    s.set_element(root, row);
    for &i in &items {
        let e = element(&mut s, &["div"], &["item", "tall"]);
        s.set_element(i, e);
    }
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 800.0, 600.0).unwrap();
    // 50% of the row's content box, 180 wide; 2em of a 10px font is 20.
    assert_eq!(bounds(&ctx, items[0])[2], 90.0);
    assert_eq!(bounds(&ctx, items[1])[0], 100.0);
    assert_eq!(bounds(&ctx, items[0])[3], 20.0);
    assert_eq!(
        s.resolved(items[0]).find(|(k, _)| *k == "color"),
        Some(("color", "red"))
    );

    // Hovering the first restyles what tested :hover: its own height, and not the other's.
    s.set_states(items[0], States(States::bit("hover").unwrap()));
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 800.0, 600.0).unwrap();
    assert_eq!(bounds(&ctx, items[0])[3], 30.0);
    assert_eq!(bounds(&ctx, items[1])[3], 20.0);

    // Without its classes the second takes no width or height from the sheet: both are unset.
    let plain = element(&mut s, &["div"], &[]);
    s.set_element(items[1], plain);
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 800.0, 600.0).unwrap();
    assert_ne!(bounds(&ctx, items[1])[2], 90.0);
    assert!(s.resolved(items[1]).next().is_none());
}

#[test]
fn a_value_it_cannot_read_is_reported_and_the_rest_applies() {
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let mut s = Styles::new();
    s.cascade_mut().push(parse(
        ".a { display: table; width: 40px }",
        None,
        &mut |_, _| None,
    ));
    let e = element(&mut s, &["div"], &["a"]);
    s.set_element(root, e);
    let errors = s.restyle(&mut ctx, root);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("display: table"));
    ctx.compute(root, 100.0, 100.0).unwrap();
    assert_eq!(bounds(&ctx, root)[2], 40.0);
}

#[test]
fn a_change_restyles_only_what_it_can_reach() {
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let rows: Vec<Node> = (0..50)
        .map(|_| ctx.create_node(Default::default()).unwrap())
        .collect();
    for &r in &rows {
        ctx.insert_before(root, r, None).unwrap();
    }
    let mut s = Styles::new();
    s.cascade_mut().push(parse(
        ".list > .row { height: 10px } .row.wide { width: 40px } .box:has(.on) { width: 99px }",
        None,
        &mut |_, _| None,
    ));
    let list = element(&mut s, &["div"], &["list", "box"]);
    s.set_element(root, list);
    for &r in &rows {
        let e = element(&mut s, &["div"], &["row"]);
        s.set_element(r, e);
    }
    s.restyle(&mut ctx, root);
    assert_eq!(s.last_restyled(), 51);

    // Its own declarations: the node alone.
    let mut e = element(&mut s, &["div"], &["row"]);
    let w = s.intern("width");
    e.inline = vec![(w, "12px".into())];
    s.set_element(rows[10], e);
    s.restyle(&mut ctx, root);
    assert_eq!(s.last_restyled(), 1);

    // A class only its own selectors test: the node alone.
    let e = element(&mut s, &["div"], &["row", "wide"]);
    s.set_element(rows[20], e);
    s.restyle(&mut ctx, root);
    assert_eq!(s.last_restyled(), 1);

    // A class a :has() above tests: the ancestor answers again.
    let e = element(&mut s, &["div"], &["row", "on"]);
    s.set_element(rows[30], e);
    s.restyle(&mut ctx, root);
    ctx.compute(root, 500.0, 900.0).unwrap();
    assert_eq!(bounds(&ctx, root)[2], 99.0);

    // A change to what it passes down reaches its children, though nothing marked them.
    let mut list = element(&mut s, &["div"], &["list", "box"]);
    let color = s.intern("color");
    list.inline = vec![(color, "blue".into())];
    s.set_element(root, list);
    s.restyle(&mut ctx, root);
    assert_eq!(
        s.computed(rows[0]).and_then(|c| c.value(color)),
        Some("blue")
    );
}

#[test]
fn a_move_restyles_what_tests_its_place() {
    // A grid of rows, each with cells, and a sheet that tests position only if asked.
    let build = |css: &str| {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(Default::default()).unwrap();
        let mut s = Styles::new();
        s.cascade_mut().push(parse(css, None, &mut |_, _| None));
        let e = element(&mut s, &["div"], &["grid"]);
        s.set_element(root, e);
        let mut rows = Vec::new();
        for _ in 0..10 {
            let r = ctx.create_node(Default::default()).unwrap();
            ctx.insert_before(root, r, None).unwrap();
            let e = element(&mut s, &["div"], &["row"]);
            s.set_element(r, e);
            for _ in 0..4 {
                let c = ctx.create_node(Default::default()).unwrap();
                ctx.insert_before(r, c, None).unwrap();
                let e = element(&mut s, &["div"], &["cell"]);
                s.set_element(c, e);
            }
            rows.push(r);
        }
        s.restyle(&mut ctx, root);
        assert_eq!(s.last_restyled(), 51);
        (ctx, s, root, rows)
    };
    let move_last_first = |ctx: &mut LayoutContext, s: &mut Styles, root: Node, rows: &[Node]| {
        ctx.insert_before(root, rows[9], Some(rows[0])).unwrap();
        s.moved(rows[9]);
        s.children_changed(root);
        s.restyle(ctx, root);
        s.last_restyled()
    };

    // Nothing tests position: the moved row and its cells.
    let (mut ctx, mut s, root, rows) = build(".row { height: 10px } .row .cell { width: 5px }");
    assert_eq!(move_last_first(&mut ctx, &mut s, root, &rows), 5);

    // The rows' places: the parent and each row, alone.
    let (mut ctx, mut s, root, rows) =
        build(".row:first-child { height: 20px } .cell { width: 5px }");
    assert_eq!(move_last_first(&mut ctx, &mut s, root, &rows), 1 + 10 + 4);
    let height = s.intern("height");
    assert_eq!(
        s.computed(rows[9]).and_then(|c| c.value(height)),
        Some("20px")
    );
    assert_eq!(s.computed(rows[0]).and_then(|c| c.value(height)), None);

    // A row's place decides its cells' styles: every row with its cells.
    let (mut ctx, mut s, root, rows) = build(".row:nth-child(odd) .cell { width: 7px }");
    assert_eq!(move_last_first(&mut ctx, &mut s, root, &rows), 51);
    ctx.compute(root, 500.0, 900.0).unwrap();
    let cell = ctx.children(rows[9]).unwrap()[0];
    assert_eq!(bounds(&ctx, cell)[2], 7.0);

    // A removal: only the old parent's children answer again.
    let (mut ctx, mut s, root, rows) = build(".row:last-child { height: 30px }");
    ctx.detach(rows[9]).unwrap();
    s.forget(rows[9]);
    s.children_changed(root);
    s.restyle(&mut ctx, root);
    assert_eq!(s.last_restyled(), 10);
    let height = s.intern("height");
    assert_eq!(
        s.computed(rows[8]).and_then(|c| c.value(height)),
        Some("30px")
    );
}

#[test]
fn mixed_lengths_follow_content_boxes_and_resize_without_restyling() {
    use crate::context::PropValue;
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let child = ctx.create_node(Default::default()).unwrap();
    ctx.insert_before(root, child, None).unwrap();
    let mut s = Styles::new();
    s.cascade_mut().push(parse(
        ".root { width: 300px; height: 200px; display: flex; flex-direction: column; align-items: flex-start; padding: 10px; border: 2px solid red } .child { width: calc(100% - 32px); height: calc(50% - 8px); flex-shrink: 0 }",
        None, &mut |_, _| None,
    ));
    for (n, c) in [(root, "root"), (child, "child")] {
        let e = element(&mut s, &["div"], &[c]);
        s.set_element(n, e);
    }
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 800.0, 600.0).unwrap();
    assert_eq!(bounds(&ctx, child), [12.0, 12.0, 244.0, 80.0]);
    ctx.apply(&[(root, 10, PropValue::Number(500.0))]).unwrap();
    ctx.compute(root, 800.0, 600.0).unwrap();
    assert_eq!(bounds(&ctx, child), [12.0, 12.0, 444.0, 80.0]);
    ctx.apply(&[(root, 53, PropValue::Number(1.0))]).unwrap();
    for width in [300.0, 300.0, 500.0, 300.0] {
        ctx.compute(root, width, 600.0).unwrap();
        assert_eq!(
            bounds(&ctx, child),
            [12.0, 12.0, width - 56.0, 80.0],
            "Cached math layout follows available width"
        );
    }

    // Replacing a math value releases its storage and restores the ordinary layout path.
    ctx.apply(&[
        (child, 10, PropValue::Number(40.0)),
        (child, 11, PropValue::Number(20.0)),
    ])
    .unwrap();
    ctx.compute(root, 800.0, 600.0).unwrap();
    assert_eq!(bounds(&ctx, child), [12.0, 12.0, 40.0, 20.0]);
    assert!(ctx.tree.as_ref().unwrap().layout.expressions.is_empty());
}

#[test]
fn deferred_math_writes_remain_transactional_and_drop_with_removed_nodes() {
    use super::layout::Units;
    use crate::context::PropValue;
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let child = ctx.create_node(Default::default()).unwrap();
    ctx.insert_before(root, child, None).unwrap();
    let math = PropValue::Math {
        source: "min(50%, 100px)",
        units: Units::default(),
    };
    assert!(
        ctx.apply(&[(child, 10, math), (child, 99999, PropValue::Number(1.0))])
            .is_err()
    );
    assert!(ctx.tree.as_ref().unwrap().layout.expressions.is_empty());
    ctx.apply(&[(root, 10, PropValue::Number(300.0)), (child, 10, math)])
        .unwrap();
    ctx.compute(root, 300.0, 200.0).unwrap();
    assert_eq!(bounds(&ctx, child)[2], 100.0);
    ctx.remove(child).unwrap();
    ctx.compute(root, 300.0, 200.0).unwrap();
    assert!(ctx.tree.as_ref().unwrap().layout.expressions.is_empty());
}

#[test]
fn mixed_insets_and_runtime_overflow_are_resolved_in_layout() {
    use super::layout::Units;
    use crate::context::PropValue;
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let child = ctx.create_node(Default::default()).unwrap();
    ctx.insert_before(root, child, None).unwrap();
    let mut s = Styles::new();
    s.cascade_mut().push(parse(".root {width:200px;height:100px;position:relative} .child {position:absolute;left:calc(25% + 3px);top:calc(50% - 4px);width:30px;height:10px}",None,&mut |_,_|None));
    for (n, c) in [(root, "root"), (child, "child")] {
        let e = element(&mut s, &["div"], &[c]);
        s.set_element(n, e);
    }
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 200.0, 100.0).unwrap();
    assert_eq!(bounds(&ctx, child), [53.0, 46.0, 30.0, 10.0]);
    ctx.apply(&[(
        child,
        10,
        PropValue::Math {
            source: "min(calc(max(50%, 10px) * 1e308), 1px)",
            units: Units::default(),
        },
    )])
    .unwrap();
    assert!(
        ctx.compute(root, 200.0, 100.0).is_err(),
        "Overflow must not be masked by min()"
    );
}

#[test]
fn sticky_inset_batches_keep_the_last_length_kind() {
    use super::layout::Units;
    use crate::context::PropValue;
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let child = ctx.create_node(Default::default()).unwrap();
    ctx.insert_before(root, child, None).unwrap();
    ctx.apply(&[
        (root, 10, PropValue::Number(200.0)),
        (root, 11, PropValue::Number(100.0)),
        (child, 29, PropValue::Enum(2)),
    ])
    .unwrap();
    let math = PropValue::Math {
        source: "calc(10% + 2px)",
        units: Units::default(),
    };
    ctx.apply(&[(child, 30, PropValue::Number(3.0)), (child, 30, math)])
        .unwrap();
    ctx.compute(root, 200.0, 100.0).unwrap();
    let id = child.id;
    assert_eq!(
        ctx.tree.as_ref().unwrap().sticky_insets(id).unwrap()[0],
        Some(12.0)
    );
    ctx.apply(&[
        (child, 30, PropValue::Number(3.0)),
        (child, 112, PropValue::Number(0.2)),
    ])
    .unwrap();
    ctx.compute(root, 200.0, 100.0).unwrap();
    assert_eq!(
        ctx.tree.as_ref().unwrap().sticky_insets(id).unwrap()[0],
        Some(20.0)
    );
    ctx.apply(&[(child, 112, PropValue::Unset)]).unwrap();
    ctx.compute(root, 200.0, 100.0).unwrap();
    assert_eq!(
        ctx.tree.as_ref().unwrap().sticky_insets(id).unwrap()[0],
        None
    );
    assert!(ctx.tree.as_ref().unwrap().sticky_expressions.is_empty());
}
#[test]
fn generated_styles_follow_origin_state_attributes_and_inheritance() {
    use super::generated::{Content, PseudoElement};
    let css = r#"
      .origin { font-size: 18px; color: blue; width: 200px; content: 'ordinary'; }
      span { font-size: 40px; color: red; }
      .origin:empty::before { content: '[' attr(title) ']'; width: 20px; }
      div:is(.origin)::after { content: url('icon.svg'); height: 12px; }
      .origin:hover::before { content: 'hover'; width: 30px; }
      .origin:hover::after { content: none; }
      .origin:has(> span) { height: 90px; }
    "#;
    for compiled in [false, true] {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(Default::default()).unwrap();
        let mut s = Styles::new();
        let sheet = parse(css, None, &mut |_, _| None);
        assert!(sheet.diagnostics.is_empty(), "{}", sheet.report(None));
        let sheet = if compiled {
            super::compiled::decode(&super::compiled::encode(&sheet)).unwrap()
        } else {
            sheet
        };
        s.cascade_mut().push(sheet);
        let mut e = element(&mut s, &["div"], &["origin"]);
        e.attributes.push((s.intern("title"), s.intern("one")));
        e.inline.push((s.intern("width"), "200px".into()));
        s.set_element(root, e.clone());
        assert!(s.restyle(&mut ctx, root).is_empty());
        let out = s.take_generated();
        assert_eq!(
            out,
            vec![
                (
                    root.raw(),
                    PseudoElement::Before,
                    Some(Content::Text("[one]".into()))
                ),
                (
                    root.raw(),
                    PseudoElement::After,
                    Some(Content::Image("icon.svg".into()))
                )
            ]
        );
        let before = ctx.create_node(Default::default()).unwrap();
        let text = ctx.create_node(Default::default()).unwrap();
        ctx.insert_before(root, before, None).unwrap();
        ctx.insert_before(before, text, None).unwrap();
        let mut box_e = element(&mut s, &["span"], &[]);
        box_e.anonymous = true;
        let mut text_e = element(&mut s, &["text"], &[]);
        text_e.anonymous = true;
        s.set_element(before, box_e);
        s.set_element(text, text_e);
        s.set_generated(before, root, PseudoElement::Before, None);
        s.children_changed(root);
        assert!(s.restyle(&mut ctx, root).is_empty());
        assert!(
            s.take_generated().is_empty(),
            ":empty ignores generated content"
        );
        ctx.compute(root, 200.0, 100.0).unwrap();
        assert_eq!(
            bounds(&ctx, before)[2],
            20.0,
            "origin inline width does not apply to pseudo"
        );
        assert_ne!(
            bounds(&ctx, root)[3],
            90.0,
            ":has ignores generated subtree"
        );
        assert_eq!(
            s.computed(text).unwrap().font_size,
            18.0,
            "generated text takes origin, not span selectors"
        );
        assert_eq!(
            s.resolved(text).find(|(k, _)| *k == "color").unwrap().1,
            "blue"
        );
        assert_eq!(s.select(&ctx, root, "span").unwrap(), vec![]);
        e.attributes[0].1 = s.intern("two");
        s.set_element(root, e);
        assert!(s.restyle(&mut ctx, root).is_empty());
        assert_eq!(
            s.take_generated(),
            vec![(
                root.raw(),
                PseudoElement::Before,
                Some(Content::Text("[two]".into()))
            )]
        );
        s.set_states(root, States(States::bit("hover").unwrap()));
        assert!(s.restyle(&mut ctx, root).is_empty());
        assert_eq!(
            s.take_generated(),
            vec![
                (
                    root.raw(),
                    PseudoElement::Before,
                    Some(Content::Text("hover".into()))
                ),
                (root.raw(), PseudoElement::After, None)
            ]
        );
        ctx.compute(root, 200.0, 100.0).unwrap();
        assert_eq!(bounds(&ctx, before)[2], 30.0);
        s.forget(before);
        s.forget(text);
        s.forget(root);
        assert!(s.take_generated().is_empty());
    }
}

#[cfg(feature = "scene")]
#[test]
fn border_side_patterns_follow_cascade_colour_changes_and_shorthand_resets() {
    use super::border::Style as BorderStyle;
    use super::paint::PaintWrite;
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let mut s = Styles::new();
    s.set_paint_output(true);
    s.cascade_mut().push(parse(
        ".box { border: 3px dashed red; border-top-style: dotted; border-color: blue; }
         .box:hover { border: 2px solid lime; }
         .box:active { border-style: dashed dotted none solid; }",
        None,
        &mut |_, _| None,
    ));
    let e = element(&mut s, &["div"], &["box"]);
    s.set_element(root, e);
    let apply = |s: &mut Styles, ctx: &mut LayoutContext| {
        assert!(s.restyle(ctx, root).is_empty());
        for (node, writes) in s.take_paint() {
            for write in writes {
                if let PaintWrite::BorderStyle { side, style } = write {
                    ctx.set_border_style(node, side, style).unwrap();
                }
            }
        }
    };
    apply(&mut s, &mut ctx);
    let id = blinc_layout::tree::LayoutNodeId::from_raw(root.raw());
    assert_eq!(
        ctx.tree.as_ref().unwrap().border_styles[&id],
        [
            BorderStyle::Dotted,
            BorderStyle::Dashed,
            BorderStyle::Dashed,
            BorderStyle::Dashed
        ]
    );
    s.set_states(root, States(States::bit("hover").unwrap()));
    apply(&mut s, &mut ctx);
    assert!(
        !ctx.tree.as_ref().unwrap().border_styles.contains_key(&id),
        "solid takes the compact default"
    );
    s.set_states(root, States(States::bit("active").unwrap()));
    apply(&mut s, &mut ctx);
    assert_eq!(
        ctx.tree.as_ref().unwrap().border_styles[&id],
        [
            BorderStyle::Dashed,
            BorderStyle::Dotted,
            BorderStyle::None,
            BorderStyle::Solid
        ]
    );
    s.set_states(root, States::default());
    apply(&mut s, &mut ctx);
    assert_eq!(
        ctx.tree.as_ref().unwrap().border_styles[&id][0],
        BorderStyle::Dotted
    );
    let plain = element(&mut s, &["div"], &[]);
    s.set_element(root, plain);
    apply(&mut s, &mut ctx);
    assert!(
        !ctx.tree.as_ref().unwrap().border_styles.contains_key(&id),
        "removing the rule resets patterns"
    );
}

#[test]
fn none_has_zero_used_width_independent_of_width_declaration_order() {
    let mut ctx = LayoutContext::new();
    let root = ctx.create_node(Default::default()).unwrap();
    let child = ctx.create_node(Default::default()).unwrap();
    ctx.set_children(root, &[child]).unwrap();
    let mut s = Styles::new();
    s.cascade_mut().push(parse(
        ".box { width:100px;height:50px;border-style:none;border-width:3px; }
        .box:hover { border-style:solid; border-left-style:none; }
        .box:active { border-left-style:dotted; }",
        None,
        &mut |_, _| None,
    ));
    let e = element(&mut s, &["div"], &["box"]);
    s.set_element(root, e);
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 100.0, 50.0).unwrap();
    assert_eq!(&bounds(&ctx, child)[..2], &[0.0, 0.0]);
    s.set_states(root, States(States::bit("active").unwrap()));
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 100.0, 50.0).unwrap();
    assert_eq!(
        &bounds(&ctx, child)[..2],
        &[3.0, 0.0],
        "a side overrides a shorthand none"
    );
    s.set_states(root, States(States::bit("hover").unwrap()));
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 100.0, 50.0).unwrap();
    assert_eq!(&bounds(&ctx, child)[..2], &[0.0, 3.0]);
    s.set_states(
        root,
        States(States::bit("hover").unwrap() | States::bit("active").unwrap()),
    );
    assert!(s.restyle(&mut ctx, root).is_empty());
    ctx.compute(root, 100.0, 50.0).unwrap();
    assert_eq!(&bounds(&ctx, child)[..2], &[3.0, 3.0]);
}
