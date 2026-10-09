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
