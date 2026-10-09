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
