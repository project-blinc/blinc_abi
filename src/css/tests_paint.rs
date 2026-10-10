use super::color;
use super::filter;
use super::gradient;
use super::paint::{Background, PaintWrite, is_paint_property, paint_writes};
use super::quantity::PaintUnits;
use super::shadow;
use super::transform;
use blinc_core::Color;
use blinc_core::layer::Gradient;

fn bytes(c: Color) -> [u32; 4] {
    [c.r, c.g, c.b, c.a].map(|v| (v * 255.0).round() as u32)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

fn rgba(text: &str) -> [u32; 4] {
    bytes(color::parse(text, None).unwrap())
}

#[test]
fn colours_read_hex_names_functions_and_mixes() {
    assert_eq!(rgba("#f00"), [255, 0, 0, 255]);
    assert_eq!(rgba("#ff000080"), [255, 0, 0, 128]);
    assert_eq!(rgba("RebeccaPurple"), [0x66, 0x33, 0x99, 255]);
    assert_eq!(rgba("tomato"), [255, 99, 71, 255]);
    assert_eq!(rgba("transparent"), [0, 0, 0, 0]);
    assert_eq!(rgba("rgb(10, 20, 30)"), [10, 20, 30, 255]);
    assert_eq!(rgba("rgb(255 0 0 / 25%)"), [255, 0, 0, 64]);
    assert_eq!(rgba("rgb(100% 50% 0%)"), [255, 128, 0, 255]);
    assert_eq!(rgba("rgb(300 0 -5)"), [255, 0, 0, 255]);
    assert_eq!(rgba("hsl(0 100% 50%)"), [255, 0, 0, 255]);
    assert_eq!(rgba("hsl(120, 100%, 25%)"), [0, 128, 0, 255]);
    assert_eq!(rgba("hsl(0.5turn 100% 50%)"), [0, 255, 255, 255]);
    assert_eq!(rgba("hsl(240deg 100% 50% / 0.5)"), [0, 0, 255, 128]);
    assert_eq!(rgba("hsl(-120 100% 50%)"), [0, 0, 255, 255]);
    assert_eq!(rgba("hsla(0, 0%, 50%, 1)"), [128, 128, 128, 255]);
    assert_eq!(rgba("color-mix(in srgb, red, blue)"), [128, 0, 128, 255]);
    assert_eq!(rgba("color-mix(in srgb, red 25%, blue)"), [64, 0, 191, 255]);
    // Percentages under 100 scale the alpha.
    assert_eq!(
        rgba("color-mix(in srgb, red 20%, blue 20%)"),
        [128, 0, 128, 102]
    );
    // A half-transparent blue weighs less than an opaque red.
    assert_eq!(
        rgba("color-mix(in srgb, red, rgba(0, 0, 255, 0.5))"),
        [170, 0, 85, 191]
    );
    assert_eq!(
        rgba("color-mix(in srgb, red, transparent)"),
        [255, 0, 0, 128]
    );
}

#[test]
fn currentcolor_is_the_nodes_own() {
    let own = Color::rgba(0.0, 1.0, 0.0, 1.0);
    assert_eq!(
        bytes(color::parse("currentcolor", Some(own)).unwrap()),
        [0, 255, 0, 255]
    );
    assert!(color::parse("currentcolor", None).is_err());
}

#[test]
fn colours_that_are_not_understood_say_why() {
    let reason = |text: &str| color::parse(text, None).unwrap_err();
    assert!(reason("notacolor").contains("expected a colour"));
    assert!(reason("lab(50% 40 59)").contains("lab() is not a colour this supports"));
    assert!(reason("rgb(1 2)").contains("three channels"));
    assert!(reason("rgb(1 2 3 4 5)").contains("three channels"));
    assert!(reason("#12").contains("3, 4, 6 or 8 digits"));
    assert!(reason("#12345g").contains("not a hex colour"));
    assert!(reason("hsl(0 1px 50%)").contains("number or a percentage"));
    assert!(reason("color-mix(in lab, red, blue)").contains("in srgb"));
    assert!(reason("rgb(1 / 2 / 3)").contains("one /"));
}

fn gradient_of(text: &str) -> Gradient {
    gradient::parse(text, None).unwrap().to_gradient()
}

fn points(g: &Gradient) -> [f32; 4] {
    match g {
        Gradient::Linear { start, end, .. } => [start.x, start.y, end.x, end.y],
        _ => panic!("not linear"),
    }
}

fn offsets(g: &Gradient) -> Vec<f32> {
    let stops = match g {
        Gradient::Linear { stops, .. } | Gradient::Radial { stops, .. } => stops,
        _ => panic!("not a colour gradient"),
    };
    stops.iter().map(|s| s.offset).collect()
}

#[test]
fn linear_gradients_run_along_the_gradient_line() {
    let eq = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| near(*x, y));
    assert!(eq(
        points(&gradient_of("linear-gradient(to right, red, blue)")),
        [0.0, 0.5, 1.0, 0.5]
    ));
    assert!(eq(
        points(&gradient_of("linear-gradient(red, blue)")),
        [0.5, 0.0, 0.5, 1.0]
    ));
    assert!(eq(
        points(&gradient_of("linear-gradient(135deg, red, blue)")),
        [0.0, 0.0, 1.0, 1.0]
    ));
    assert!(eq(
        points(&gradient_of("linear-gradient(to top left, red, blue)")),
        [1.0, 1.0, 0.0, 0.0]
    ));
    assert!(eq(
        points(&gradient_of("linear-gradient(0.25turn, red, blue)")),
        [0.0, 0.5, 1.0, 0.5]
    ));
}

#[test]
fn gradient_stops_are_spread_as_css_spreads_them() {
    let of = |t: &str| offsets(&gradient_of(t));
    let eq = |a: Vec<f32>, b: Vec<f32>| {
        a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| near(*x, *y))
    };
    assert!(eq(of("linear-gradient(red, blue)"), vec![0.0, 1.0]));
    assert!(eq(
        of("linear-gradient(red, yellow, blue)"),
        vec![0.0, 0.5, 1.0]
    ));
    assert!(eq(
        of("linear-gradient(red, yellow, lime, blue)"),
        vec![0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]
    ));
    assert!(eq(of("linear-gradient(red 10%, blue 90%)"), vec![0.1, 0.9]));
    assert!(eq(
        of("linear-gradient(red, lime 25% 75%, blue)"),
        vec![0.0, 0.25, 0.75, 1.0]
    ));
    // Never backwards, and never past the unit range.
    assert!(eq(
        of("linear-gradient(red 60%, lime 20%, blue)"),
        vec![0.6, 0.6, 1.0]
    ));
    assert!(eq(of("linear-gradient(red, blue 150%)"), vec![0.0, 1.0]));
}

#[test]
fn radial_gradients_reach_the_farthest_corner() {
    let radial = |t: &str| match gradient_of(t) {
        Gradient::Radial { center, radius, .. } => [center.x, center.y, radius],
        _ => panic!("not radial"),
    };
    let eq = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| near(*x, y));
    assert!(eq(
        radial("radial-gradient(circle, white, black)"),
        [0.5, 0.5, std::f32::consts::FRAC_1_SQRT_2]
    ));
    assert!(eq(
        radial("radial-gradient(circle at 25% 75%, red, blue)"),
        [0.25, 0.75, 0.75f32.hypot(0.75)]
    ));
    assert!(eq(
        radial("radial-gradient(ellipse at left top, red, blue)"),
        [0.0, 0.0, std::f32::consts::SQRT_2]
    ));
    assert!(matches!(
        gradient_of("radial-gradient(red, blue)"),
        Gradient::Radial { .. }
    ));
}

#[test]
fn gradients_that_are_not_understood_say_why() {
    let reason = |t: &str| gradient::parse(t, None).unwrap_err();
    assert!(reason("linear-gradient(red)").contains("two colour stops"));
    assert!(reason("conic-gradient(red, blue)").contains("conic-gradient"));
    assert!(reason("radial-gradient(closest-side, red, blue)").contains("closest-side"));
    assert!(reason("linear-gradient(to middle, red, blue)").contains("top, bottom, left or right"));
    assert!(reason("linear-gradient(red 10px, blue)").contains("number or a percentage"));
    assert!(reason("red").contains("expected a gradient"));
    assert!(gradient::is_gradient(
        "repeating-linear-gradient(red, blue)"
    ));
    assert!(!gradient::is_gradient("rgb(1, 2, 3)"));
}

fn matrix(text: &str) -> [f32; 6] {
    transform::parse(text, &PaintUnits::default())
        .unwrap()
        .elements
}

#[test]
fn transforms_compose_in_order() {
    let eq = |a: [f32; 6], b: [f32; 6]| a.iter().zip(b).all(|(x, y)| near(*x, y));
    assert_eq!(matrix("none"), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(
        matrix("translate(10px, 20px)"),
        [1.0, 0.0, 0.0, 1.0, 10.0, 20.0]
    );
    assert_eq!(
        matrix("translateX(5px) translateY(-6px)"),
        [1.0, 0.0, 0.0, 1.0, 5.0, -6.0]
    );
    assert_eq!(matrix("scale(2)"), [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    assert_eq!(matrix("scale(2, 3)"), [2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
    assert_eq!(matrix("scale(50%)"), [0.5, 0.0, 0.0, 0.5, 0.0, 0.0]);
    assert!(eq(matrix("rotate(90deg)"), [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]));
    assert!(eq(matrix("skewX(45deg)"), [1.0, 0.0, 1.0, 1.0, 0.0, 0.0]));
    assert_eq!(
        matrix("matrix(1, 2, 3, 4, 5, 6)"),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    );
    // The first function is the outermost: scale, then move the scaled box.
    assert_eq!(
        matrix("translate(10px, 0) scale(2)"),
        [2.0, 0.0, 0.0, 2.0, 10.0, 0.0]
    );
    assert_eq!(
        matrix("scale(2) translate(10px, 0)"),
        [2.0, 0.0, 0.0, 2.0, 20.0, 0.0]
    );
    let units = PaintUnits {
        font_size: 10.0,
        ..Default::default()
    };
    assert_eq!(
        transform::parse("translate(2em, 0)", &units)
            .unwrap()
            .elements[4],
        20.0
    );
}

#[test]
fn transforms_that_are_not_understood_say_why() {
    let reason = |t: &str| transform::parse(t, &PaintUnits::default()).unwrap_err();
    assert!(reason("translate(50%)").contains("% of the element's own size"));
    assert!(reason("perspective(1px)").contains("not a 2D transform"));
    assert!(reason("rotate(10)").contains("expected an angle"));
    assert!(reason("scale()").contains("expected a number"));
    assert!(reason("matrix(1, 2)").contains("takes 6 arguments"));
    assert!(reason("scale(1, 2, 3)").contains("takes 1 or 2 arguments"));
    assert!(reason("x").contains("expected a transform function"));
}

#[test]
fn filters_become_one_value_and_the_last_function_wins() {
    let units = PaintUnits::default();
    let f = filter::parse("blur(4px) grayscale(50%)", &units).unwrap();
    assert_eq!((f.blur, f.grayscale, f.brightness), (4.0, 0.5, 1.0));
    let f = filter::parse(
        "brightness(1.2) contrast(80%) saturate(2) sepia(1) invert(0.5)",
        &units,
    )
    .unwrap();
    assert_eq!(
        (f.brightness, f.contrast, f.saturate, f.sepia, f.invert),
        (1.2, 0.8, 2.0, 1.0, 0.5)
    );
    assert!(
        (filter::parse("hue-rotate(0.5turn)", &units)
            .unwrap()
            .hue_rotate
            - 180.0)
            .abs()
            < 1e-4
    );
    assert_eq!(
        filter::parse("blur(1px) blur(2px)", &units).unwrap().blur,
        2.0
    );
    assert_eq!(
        filter::parse("grayscale(3)", &units).unwrap().grayscale,
        1.0,
        "clamped"
    );
    assert!(filter::parse("none", &units).unwrap().is_identity());
    let drop = filter::parse("drop-shadow(2px 4px 6px rgba(0, 0, 0, 0.5))", &units)
        .unwrap()
        .drop_shadow
        .unwrap();
    assert_eq!(
        (drop.offset_x, drop.offset_y, drop.blur, drop.color.a),
        (2.0, 4.0, 6.0, 0.5)
    );
    assert!(
        filter::parse("opacity(0.5)", &units)
            .unwrap_err()
            .contains("use opacity")
    );
    assert!(
        filter::parse("url(#f)", &units)
            .unwrap_err()
            .contains("not a filter")
    );
    assert!(
        filter::parse("drop-shadow(1px)", &units)
            .unwrap_err()
            .contains("x and y offset")
    );
    assert!(
        filter::parse("drop-shadow(inset 1px 2px)", &units)
            .unwrap_err()
            .contains("inset is for box-shadow")
    );
}

#[test]
fn shadows_split_into_outer_and_inset_layers_in_order() {
    let units = PaintUnits::default();
    let (outer, inner) = shadow::parse(
        "0 1px 2px red, inset 0 2px 4px 1px rgba(0,0,0,0.5), 4px 5px blue",
        &units,
    )
    .unwrap();
    assert_eq!(outer.len(), 2);
    assert_eq!(
        (outer[0].offset_y, outer[0].blur, outer[0].spread),
        (1.0, 2.0, 0.0)
    );
    assert_eq!(
        (outer[1].offset_x, outer[1].offset_y, outer[1].blur),
        (4.0, 5.0, 0.0)
    );
    assert_eq!(
        (inner[0].blur, inner[0].spread, inner[0].color.a),
        (4.0, 1.0, 0.5)
    );
    let (none, _) = shadow::parse("none", &units).unwrap();
    assert!(none.is_empty());
    // With no colour, a shadow takes the node's own, or black.
    let own = PaintUnits {
        color: Some(Color::rgba(0.0, 1.0, 0.0, 1.0)),
        ..Default::default()
    };
    assert_eq!(shadow::parse("1px 2px", &own).unwrap().0[0].color.g, 1.0);
    assert_eq!(shadow::parse("1px 2px", &units).unwrap().0[0].color.a, 1.0);
    assert!(
        shadow::parse("1px", &units)
            .unwrap_err()
            .contains("x and y offset")
    );
    assert!(
        shadow::parse("1px 2px -3px", &units)
            .unwrap_err()
            .contains("negative")
    );
}

fn writes(name: &str, value: &str) -> Vec<PaintWrite> {
    paint_writes(name, Some(value), &PaintUnits::default())
        .unwrap()
        .unwrap()
}

#[test]
fn a_background_is_a_colour_none_a_gradient_or_glass() {
    assert!(
        matches!(&writes("background", "hsl(120 100% 25%)")[0], PaintWrite::Background(Background::Solid(c)) if bytes(*c) == [0, 128, 0, 255])
    );
    assert!(matches!(
        &writes("background", "none")[0],
        PaintWrite::Background(Background::None)
    ));
    assert!(matches!(
        &writes("background-color", "transparent")[0],
        PaintWrite::Background(Background::None)
    ));
    assert!(matches!(
        &writes("background-image", "linear-gradient(red, blue)")[0],
        PaintWrite::Background(Background::Gradient(_))
    ));
    let own = PaintUnits {
        color: Some(Color::rgba(0.0, 1.0, 0.0, 1.0)),
        ..Default::default()
    };
    let w = paint_writes(
        "background",
        Some("linear-gradient(currentcolor, black)"),
        &own,
    )
    .unwrap()
    .unwrap();
    assert!(
        matches!(&w[0], PaintWrite::Background(Background::Gradient(Gradient::Linear { stops, .. })) if stops[0].color.g == 1.0)
    );
    let reason = |v: &str| {
        paint_writes("background", Some(v), &PaintUnits::default())
            .unwrap()
            .unwrap_err()
    };
    assert!(reason("url(a.png)").contains("url()"));
    assert!(reason("not a colour").contains("expected a colour"));
    assert!(reason("currentcolor").contains("currentcolor"));
}

#[test]
fn glass_reads_its_settings_from_the_nodes_declarations() {
    let glass = |declared: &[(&str, &str)]| {
        let units = PaintUnits {
            declared,
            ..Default::default()
        };
        match paint_writes("background", Some("glass"), &units)
            .unwrap()
            .unwrap()
            .remove(0)
        {
            PaintWrite::Background(Background::Glass(style, effects)) => (style, effects),
            other => panic!("not glass: {other:?}"),
        }
    };
    let (style, effects) = glass(&[]);
    assert_eq!(style.blur, 12.0);
    assert_eq!((style.tint.r, style.tint.a), (1.0, 0.1));
    assert!(!style.simple && style.noise == 0.0);
    assert_eq!(
        (effects.aberration, effects.bevel, effects.inset),
        (0.3, 1.0, false)
    );
    let (style, effects) = glass(&[
        ("glass-blur", "20px"),
        ("glass-tint", "rgba(0, 0, 255, 0.4)"),
        ("glass-aberration", "0%"),
        ("glass-bevel", "50%"),
        ("glass-noise", "0.25"),
        ("glass-mode", "Frosted"),
        ("glass-curvature", "inset"),
    ]);
    assert_eq!(style.blur, 20.0);
    assert_eq!((style.tint.b, style.tint.a), (1.0, 0.4));
    assert!(style.simple);
    assert_eq!(style.noise, 0.25);
    assert_eq!(
        (effects.aberration, effects.bevel, effects.inset),
        (0.0, 0.5, true)
    );
    let (style, _) = glass(&[("glass-blur", "-3px"), ("glass-mode", "milky")]);
    assert_eq!(style.blur, 12.0, "a bad setting stays at its default");
    assert!(!style.simple);
    let reason = |n: &str, v: &str| {
        paint_writes(n, Some(v), &PaintUnits::default())
            .unwrap()
            .unwrap_err()
    };
    assert!(reason("glass-blur", "-1px").contains("nonnegative"));
    assert!(reason("glass-aberration", "2").contains("0 to 1"));
    assert!(reason("glass-mode", "milky").contains("liquid or frosted"));
    assert!(reason("glass-curvature", "flat").contains("inset or outset"));
    assert!(
        paint_writes(
            "glass-tint",
            Some("hsl(0 0% 100% / 0.1)"),
            &PaintUnits::default()
        )
        .unwrap()
        .unwrap()
        .is_empty()
    );
}

#[test]
fn the_other_paint_properties_read_their_values() {
    assert!(matches!(writes("opacity", "50%")[0], PaintWrite::Opacity(o) if near(o, 0.5)));
    assert!(matches!(writes("opacity", "3")[0], PaintWrite::Opacity(o) if o == 1.0));
    assert!(matches!(
        writes("visibility", "hidden")[0],
        PaintWrite::Visible(false)
    ));
    assert!(matches!(
        writes("visibility", "visible")[0],
        PaintWrite::Visible(true)
    ));
    assert!(
        matches!(writes("border-radius", "4px 8px")[0], PaintWrite::BorderRadius(r) if r == [4.0, 8.0, 4.0, 8.0])
    );
    assert!(
        matches!(writes("border-radius", "1px 2px 3px")[0], PaintWrite::BorderRadius(r) if r == [1.0, 2.0, 3.0, 2.0])
    );
    assert!(
        matches!(writes("border-radius", "-4px")[0], PaintWrite::BorderRadius(r) if r == [0.0; 4])
    );
    assert!(
        matches!(writes("corner-shape", "squircle")[0], PaintWrite::CornerShape { shapes, locked: false } if shapes == [2.0; 4])
    );
    assert!(
        matches!(writes("corner-shape", "round locked")[0], PaintWrite::CornerShape { shapes, locked: true } if shapes == [1.0; 4])
    );
    assert!(
        matches!(writes("corner-shape", "round bevel scoop")[0], PaintWrite::CornerShape { shapes, .. } if shapes == [1.0, 0.0, -1.0, 0.0])
    );
    assert!(
        matches!(writes("corner-shape", "superellipse(3)")[0], PaintWrite::CornerShape { shapes, .. } if shapes == [3.0; 4])
    );
    assert!(
        matches!(&writes("border-color", "red")[0], PaintWrite::BorderColor(Some(c)) if bytes(*c) == [255, 0, 0, 255])
    );
    assert!(
        matches!(&writes("color", "hsl(0 100% 50%)")[0], PaintWrite::TextColor(Some(c)) if bytes(*c) == [255, 0, 0, 255])
    );
    assert!(
        matches!(&writes("box-shadow", "0 1px 2px red")[0], PaintWrite::Shadows { outer, inner } if outer.len() == 1 && inner.is_empty())
    );
    assert!(
        matches!(&writes("transform", "scale(2)")[0], PaintWrite::Transform(t) if t.elements[0] == 2.0)
    );
    assert!(matches!(&writes("filter", "blur(3px)")[0], PaintWrite::Filter(f) if f.blur == 3.0));
    assert!(matches!(
        &writes("mask-image", "linear-gradient(black, transparent)")[0],
        PaintWrite::Mask(Some(_))
    ));
    assert!(matches!(
        &writes("mask-image", "none")[0],
        PaintWrite::Mask(None)
    ));
    assert!(writes("background-size", "cover").is_empty());
}

#[test]
fn values_that_are_not_understood_are_errors_not_panics() {
    let reason = |n: &str, v: &str| {
        paint_writes(n, Some(v), &PaintUnits::default())
            .unwrap()
            .unwrap_err()
    };
    assert!(reason("border-radius", "50%").contains("% is not supported"));
    assert!(reason("border-radius", "1px / 2px").contains("elliptical"));
    assert!(reason("border-radius", "1px 2px 3px 4px 5px").contains("one to four values"));
    assert!(reason("corner-shape", "wavy").contains("expected round, squircle"));
    assert!(reason("corner-shape", "").contains("one to four shapes"));
    assert!(reason("mask-image", "red").contains("gradient or none"));
    assert!(reason("opacity", "x").contains("number or a percentage"));
}

#[test]
fn unsetting_a_property_writes_its_default() {
    let unset = |n: &str| {
        paint_writes(n, None, &PaintUnits::default())
            .unwrap()
            .unwrap()
    };
    assert!(matches!(
        unset("background")[0],
        PaintWrite::Background(Background::None)
    ));
    assert!(matches!(unset("color")[0], PaintWrite::TextColor(None)));
    assert!(matches!(unset("opacity")[0], PaintWrite::Opacity(o) if o == 1.0));
    assert!(matches!(unset("visibility")[0], PaintWrite::Visible(true)));
    assert!(matches!(unset("border-radius")[0], PaintWrite::BorderRadius(r) if r == [0.0; 4]));
    assert!(
        matches!(unset("corner-shape")[0], PaintWrite::CornerShape { shapes, locked: false } if shapes == [1.0; 4])
    );
    assert!(
        matches!(&unset("box-shadow")[0], PaintWrite::Shadows { outer, inner } if outer.is_empty() && inner.is_empty())
    );
    assert!(
        matches!(&unset("transform")[0], PaintWrite::Transform(t) if t.elements == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    );
    assert!(matches!(&unset("filter")[0], PaintWrite::Filter(f) if f.is_identity()));
    assert!(matches!(unset("mask-image")[0], PaintWrite::Mask(None)));
    assert!(unset("glass-blur").is_empty());
}

#[test]
fn only_paint_properties_are_paint_writes() {
    for name in [
        "background",
        "color",
        "transform",
        "glass-tint",
        "-webkit-mask-image",
    ] {
        assert!(is_paint_property(name), "{name}");
    }
    for name in [
        "width",
        "font-size",
        "cursor",
        "display",
        "text-decoration",
        "x",
    ] {
        assert!(!is_paint_property(name), "{name}");
        assert!(
            paint_writes(name, Some("1"), &PaintUnits::default()).is_none(),
            "{name}"
        );
    }
}

mod restyle {
    use super::super::cascade::Element;
    use super::super::paint::{Background, PaintWrite};
    use super::super::styled::Styles;
    use super::super::{MediaEnvironment, parse};
    use crate::context::{LayoutContext, Node};
    use blinc_core::layer::Gradient;

    fn setup(sheet: &str) -> (LayoutContext, Styles, Node, Node) {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(Default::default()).unwrap();
        let child = ctx.create_node(Default::default()).unwrap();
        ctx.insert_before(root, child, None).unwrap();
        let mut s = Styles::new();
        s.set_paint_output(true);
        s.set_environment(MediaEnvironment {
            width: 800.0,
            height: 600.0,
            dark: false,
        });
        s.cascade_mut().push(parse(sheet, None, &mut |_, _| None));
        let plain = element(&mut s, &[]);
        s.set_element(root, plain);
        (ctx, s, root, child)
    }

    fn element(s: &mut Styles, classes: &[&str]) -> Element {
        Element {
            types: vec![s.intern("div")],
            classes: classes.iter().map(|c| s.intern(c)).collect(),
            ..Default::default()
        }
    }

    fn writes_of(taken: Vec<(Node, Vec<PaintWrite>)>, node: Node) -> Vec<PaintWrite> {
        taken
            .into_iter()
            .filter(|(n, _)| *n == node)
            .flat_map(|(_, w)| w)
            .collect()
    }

    #[test]
    fn paint_reaches_the_host_typed_and_resolved_against_the_node() {
        let (mut ctx, mut s, root, child) = setup(
            ".a { font-size: 10px; color: rgb(0 255 0); border-radius: 2em;
                  box-shadow: 0 1px 2px currentcolor; background: linear-gradient(red, blue) }",
        );
        let e = element(&mut s, &["a"]);
        s.set_element(child, e);
        assert!(s.restyle(&mut ctx, root).is_empty());
        let w = writes_of(s.take_paint(), child);
        assert!(w.iter().any(|w| matches!(
            w,
            PaintWrite::Background(Background::Gradient(Gradient::Linear { .. }))
        )));
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::BorderRadius(r) if *r == [20.0; 4])),
            "2em of this node's 10px"
        );
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::TextColor(Some(c)) if c.g == 1.0))
        );
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::Shadows { outer, .. } if outer[0].color.g == 1.0)),
            "currentcolor is the node's own"
        );
        // Nothing changed, so nothing is handed over.
        assert!(s.restyle(&mut ctx, root).is_empty());
        assert!(s.take_paint().is_empty());
    }

    #[test]
    fn a_host_that_does_not_ask_for_paint_is_handed_none() {
        let (mut ctx, mut s, root, child) = setup(".a { background: red; opacity: 0.5 }");
        s.set_paint_output(false);
        let e = element(&mut s, &["a"]);
        s.set_element(child, e);
        assert!(s.restyle(&mut ctx, root).is_empty());
        assert!(s.take_paint().is_empty());
        // The declarations are still there to read, as they always were.
        assert!(s.resolved(child).any(|(k, _)| k == "background"));
    }

    #[test]
    fn what_a_node_loses_is_unset_and_what_it_gains_is_written() {
        let (mut ctx, mut s, root, child) =
            setup(".a { background: red; border-radius: 4px; color: lime } .b { opacity: 0.5 }");
        let e = element(&mut s, &["a"]);
        s.set_element(child, e);
        s.restyle(&mut ctx, root);
        s.take_paint();
        let e = element(&mut s, &["b"]);
        s.set_element(child, e);
        assert!(s.restyle(&mut ctx, root).is_empty());
        let w = writes_of(s.take_paint(), child);
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::Background(Background::None)))
        );
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::BorderRadius(r) if *r == [0.0; 4]))
        );
        assert!(w.iter().any(|w| matches!(w, PaintWrite::TextColor(None))));
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::Opacity(o) if *o == 0.5))
        );
        // A node with no paint left is handed nothing more.
        let e = element(&mut s, &[]);
        s.set_element(child, e);
        s.restyle(&mut ctx, root);
        let w = writes_of(s.take_paint(), child);
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::Opacity(o) if *o == 1.0))
        );
        let e = element(&mut s, &[]);
        s.set_element(child, e);
        s.restyle(&mut ctx, root);
        assert!(s.take_paint().is_empty());
    }

    #[test]
    fn glass_reads_the_declarations_beside_it() {
        let (mut ctx, mut s, root, child) =
            setup(".g { background: glass; glass-blur: 30px; glass-mode: frosted }");
        let e = element(&mut s, &["g"]);
        s.set_element(child, e);
        assert!(s.restyle(&mut ctx, root).is_empty());
        let w = writes_of(s.take_paint(), child);
        assert!(w.iter().any(|w| matches!(
            w,
            PaintWrite::Background(Background::Glass(style, _)) if style.blur == 30.0 && style.simple
        )));
    }

    #[test]
    fn a_value_that_cannot_be_read_is_reported_and_the_rest_still_applies() {
        let (mut ctx, mut s, root, child) =
            setup(".bad { background: not-a-colour; opacity: 0.25; transform: wobble(1) }");
        let e = element(&mut s, &["bad"]);
        s.set_element(child, e);
        let errors = s.restyle(&mut ctx, root);
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("background: not-a-colour: expected a colour")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("transform: wobble(1): wobble() is not a 2D transform")),
            "{errors:?}"
        );
        let w = writes_of(s.take_paint(), child);
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::Opacity(o) if *o == 0.25))
        );
        assert!(!w.iter().any(|w| matches!(w, PaintWrite::Background(_))));
    }

    #[test]
    fn a_nodes_own_colour_change_reaches_a_currentcolor_it_did_not_restate() {
        let (mut ctx, mut s, root, child) = setup(
            ".a { box-shadow: 0 1px 2px currentcolor } .red { color: red } .blue { color: blue }",
        );
        let e = element(&mut s, &["a", "red"]);
        s.set_element(child, e);
        s.restyle(&mut ctx, root);
        let w = writes_of(s.take_paint(), child);
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::Shadows { outer, .. } if outer[0].color.r == 1.0))
        );
        let e = element(&mut s, &["a", "blue"]);
        s.set_element(child, e);
        s.restyle(&mut ctx, root);
        let w = writes_of(s.take_paint(), child);
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::Shadows { outer, .. } if outer[0].color.b == 1.0)),
            "the shadow follows the colour though its own declaration did not change"
        );
    }
}

mod borders {
    use super::super::layout::{Units, id::*, is_layout_property, layout_writes};
    use super::super::paint::{PaintWrite, is_paint_property, paint_writes};
    use super::super::quantity::PaintUnits;
    use crate::context::PropValue;
    use blinc_core::Color;

    fn widths(name: &str, value: Option<&str>) -> Vec<(i32, Option<f32>)> {
        layout_writes(name, value, &Units::default())
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|(id, v)| {
                (
                    id,
                    match v {
                        PropValue::Number(n) => Some(n),
                        PropValue::Unset => None,
                        _ => panic!("not a width"),
                    },
                )
            })
            .collect()
    }

    fn sides(width: Option<f32>) -> Vec<(i32, Option<f32>)> {
        [
            BORDER_TOP_WIDTH,
            BORDER_RIGHT_WIDTH,
            BORDER_BOTTOM_WIDTH,
            BORDER_LEFT_WIDTH,
        ]
        .map(|id| (id, width))
        .to_vec()
    }

    #[test]
    fn a_border_shorthand_gives_each_side_its_width() {
        assert_eq!(widths("border", Some("2px solid red")), sides(Some(2.0)));
        assert_eq!(
            widths("border", Some("red 4px")),
            sides(Some(4.0)),
            "any order"
        );
        assert_eq!(widths("border", Some("thin")), sides(Some(1.0)));
        assert_eq!(widths("border", Some("thick dashed")), sides(Some(5.0)));
        assert_eq!(
            widths("border", Some("solid red")),
            sides(Some(3.0)),
            "medium when none is named"
        );
        assert_eq!(widths("border", Some("none")), sides(Some(0.0)));
        assert_eq!(widths("border", Some("2px hidden red")), sides(Some(0.0)));
        assert_eq!(
            widths("border", Some("0.5em solid")),
            sides(Some(8.0)),
            "em of the default 16px"
        );
        assert_eq!(widths("border", None), sides(None));
        assert_eq!(
            widths("border-left", Some("3px solid red")),
            vec![(BORDER_LEFT_WIDTH, Some(3.0))]
        );
        assert_eq!(
            widths("border-top", Some("none")),
            vec![(BORDER_TOP_WIDTH, Some(0.0))]
        );
        assert_eq!(
            widths("border-bottom", None),
            vec![(BORDER_BOTTOM_WIDTH, None)]
        );
    }

    #[test]
    fn border_widths_take_keywords_and_reject_negatives() {
        assert_eq!(
            widths("border-width", Some("thin medium thick 2px")),
            vec![
                (BORDER_TOP_WIDTH, Some(1.0)),
                (BORDER_RIGHT_WIDTH, Some(3.0)),
                (BORDER_BOTTOM_WIDTH, Some(5.0)),
                (BORDER_LEFT_WIDTH, Some(2.0)),
            ]
        );
        assert_eq!(
            widths("border-top-width", Some("thick")),
            vec![(BORDER_TOP_WIDTH, Some(5.0))]
        );
        assert!(
            layout_writes("border-top-width", Some("-1px"), &Units::default())
                .unwrap()
                .is_err()
        );
        assert!(
            layout_writes("border", Some("-2px solid"), &Units::default())
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn border_style_none_removes_the_border_on_its_sides() {
        assert_eq!(widths("border-style", Some("solid")), vec![]);
        assert_eq!(widths("border-style", Some("none")), sides(Some(0.0)));
        assert_eq!(
            widths("border-style", Some("none solid")),
            vec![
                (BORDER_TOP_WIDTH, Some(0.0)),
                (BORDER_BOTTOM_WIDTH, Some(0.0))
            ]
        );
        assert_eq!(widths("border-style", None), sides(None));
    }

    #[test]
    fn a_border_is_both_layout_and_paint() {
        for name in ["border", "border-top", "border-left"] {
            assert!(
                is_layout_property(name) && is_paint_property(name),
                "{name}"
            );
        }
        assert!(is_layout_property("border-style") && !is_paint_property("border-style"));
        assert!(!is_layout_property("border-color") && is_paint_property("border-color"));
        assert!(is_paint_property("outline") && !is_layout_property("outline"));
    }

    fn paint(name: &str, value: Option<&str>) -> Vec<PaintWrite> {
        paint_writes(name, value, &PaintUnits::default())
            .unwrap()
            .unwrap()
    }

    fn bytes(c: Color) -> [u32; 4] {
        [c.r, c.g, c.b, c.a].map(|v| (v * 255.0).round() as u32)
    }

    #[test]
    fn border_colours_are_shared_or_per_side() {
        assert!(
            matches!(&paint("border", Some("1px solid red"))[..], [PaintWrite::BorderColor(Some(c))] if bytes(*c) == [255, 0, 0, 255])
        );
        assert!(
            matches!(&paint("border-color", Some("blue"))[..], [PaintWrite::BorderColor(Some(c))] if bytes(*c) == [0, 0, 255, 255])
        );
        assert!(
            matches!(&paint("border-left", Some("3px solid lime"))[..], [PaintWrite::BorderSideColor { side: 3, color: Some(c) }] if bytes(*c) == [0, 255, 0, 255])
        );
        assert!(matches!(
            &paint("border-top-color", Some("red"))[..],
            [PaintWrite::BorderSideColor {
                side: 0,
                color: Some(_)
            }]
        ));
        let four = paint("border-color", Some("red blue"));
        let sides: Vec<(usize, [u32; 4])> = four
            .iter()
            .map(|w| match w {
                PaintWrite::BorderSideColor {
                    side,
                    color: Some(c),
                } => (*side, bytes(*c)),
                _ => panic!("not a side colour"),
            })
            .collect();
        assert_eq!(
            sides,
            vec![
                (0, [255, 0, 0, 255]),
                (1, [0, 0, 255, 255]),
                (2, [255, 0, 0, 255]),
                (3, [0, 0, 255, 255])
            ]
        );
        // A shorthand with no colour takes the node's own, or black.
        let own = PaintUnits {
            color: Some(Color::rgba(0.0, 1.0, 0.0, 1.0)),
            ..Default::default()
        };
        let w = paint_writes("border", Some("2px solid"), &own)
            .unwrap()
            .unwrap();
        assert!(matches!(&w[..], [PaintWrite::BorderColor(Some(c))] if c.g == 1.0));
        assert!(
            matches!(&paint("border", Some("2px solid"))[..], [PaintWrite::BorderColor(Some(c))] if bytes(*c) == [0, 0, 0, 255])
        );
    }

    #[test]
    fn unsetting_a_border_colour_takes_it_back() {
        assert!(matches!(
            &paint("border", None)[..],
            [PaintWrite::BorderColor(None)]
        ));
        assert!(matches!(
            &paint("border-color", None)[..],
            [PaintWrite::BorderColor(None)]
        ));
        assert!(matches!(
            &paint("border-bottom", None)[..],
            [PaintWrite::BorderSideColor {
                side: 2,
                color: None
            }]
        ));
        assert!(matches!(
            &paint("border-right-color", None)[..],
            [PaintWrite::BorderSideColor {
                side: 1,
                color: None
            }]
        ));
    }

    #[test]
    fn outlines_have_a_width_a_colour_and_an_offset() {
        let w = paint("outline", Some("2px solid red"));
        assert!(
            matches!(&w[..], [PaintWrite::OutlineWidth(2.0), PaintWrite::OutlineColor(Some(c))] if bytes(*c) == [255, 0, 0, 255])
        );
        assert!(matches!(
            &paint("outline", Some("auto red"))[..],
            [
                PaintWrite::OutlineWidth(3.0),
                PaintWrite::OutlineColor(Some(_))
            ]
        ));
        assert!(matches!(
            &paint("outline", Some("none"))[..],
            [PaintWrite::OutlineWidth(0.0), _]
        ));
        assert!(matches!(
            &paint("outline-width", Some("thick"))[..],
            [PaintWrite::OutlineWidth(5.0)]
        ));
        assert!(matches!(
            &paint("outline-color", Some("hsl(0 100% 50%)"))[..],
            [PaintWrite::OutlineColor(Some(_))]
        ));
        assert!(
            matches!(&paint("outline-offset", Some("-2px"))[..], [PaintWrite::OutlineOffset(o)] if *o == -2.0)
        );
        assert!(matches!(
            &paint("outline", None)[..],
            [
                PaintWrite::OutlineWidth(0.0),
                PaintWrite::OutlineColor(None)
            ]
        ));
        assert!(matches!(
            &paint("outline-offset", None)[..],
            [PaintWrite::OutlineOffset(0.0)]
        ));
    }

    #[test]
    fn borders_and_outlines_that_are_not_understood_say_why() {
        let reason = |n: &str, v: &str| {
            paint_writes(n, Some(v), &PaintUnits::default())
                .unwrap()
                .unwrap_err()
        };
        assert!(reason("border", "1px solid notacolor").contains("expected a colour"));
        assert!(
            reason("border-color", "red blue green yellow pink").contains("one to four values")
        );
        assert!(reason("border-left-color", "nope").contains("expected a colour"));
        assert!(reason("outline", "-1px solid").contains("cannot be negative"));
        assert!(reason("outline-width", "wide").contains("expected a width"));
        assert!(reason("outline-offset", "50%").contains("percentage"));
    }
}

mod bordered_restyle {
    use super::super::cascade::Element;
    use super::super::paint::PaintWrite;
    use super::super::styled::Styles;
    use super::super::{MediaEnvironment, parse};
    use crate::context::LayoutContext;

    #[test]
    fn one_border_declaration_gives_layout_space_and_a_colour() {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(Default::default()).unwrap();
        let child = ctx.create_node(Default::default()).unwrap();
        let inner = ctx.create_node(Default::default()).unwrap();
        ctx.insert_before(root, child, None).unwrap();
        ctx.insert_before(child, inner, None).unwrap();
        let mut s = Styles::new();
        s.set_paint_output(true);
        s.set_environment(MediaEnvironment {
            width: 800.0,
            height: 600.0,
            dark: false,
        });
        s.cascade_mut().push(parse(
            ".box { width: 100px; height: 60px; border: 4px solid red; outline: 2px solid blue; outline-offset: 3px }
             .dot { width: 10px; height: 10px }
             .left { border-left: 6px solid lime }",
            None,
            &mut |_, _| None,
        ));
        let el = |s: &mut Styles, classes: &[&str]| Element {
            types: vec![s.intern("div")],
            classes: classes.iter().map(|c| s.intern(c)).collect(),
            ..Default::default()
        };
        let plain = el(&mut s, &[]);
        s.set_element(root, plain);
        let boxed = el(&mut s, &["box"]);
        s.set_element(child, boxed);
        let dot = el(&mut s, &["dot"]);
        s.set_element(inner, dot);
        assert!(s.restyle(&mut ctx, root).is_empty());
        ctx.compute(root, 800.0, 600.0).unwrap();
        let mut out = [0.0; 4];
        ctx.read_bounds(&[inner], &mut out).unwrap();
        assert_eq!(
            (out[0], out[1]),
            (4.0, 4.0),
            "the border insets its content"
        );
        let w: Vec<PaintWrite> = s
            .take_paint()
            .into_iter()
            .filter(|(n, _)| *n == child)
            .flat_map(|(_, w)| w)
            .collect();
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::BorderColor(Some(c)) if c.r == 1.0))
        );
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::OutlineWidth(o) if *o == 2.0))
        );
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::OutlineColor(Some(c)) if c.b == 1.0))
        );
        assert!(
            w.iter()
                .any(|w| matches!(w, PaintWrite::OutlineOffset(o) if *o == 3.0))
        );

        // A side's own border stands over the shared one, in layout and in paint.
        let both = el(&mut s, &["box", "left"]);
        s.set_element(child, both);
        assert!(s.restyle(&mut ctx, root).is_empty());
        ctx.compute(root, 800.0, 600.0).unwrap();
        ctx.read_bounds(&[inner], &mut out).unwrap();
        assert_eq!((out[0], out[1]), (6.0, 4.0));
        let w: Vec<PaintWrite> = s
            .take_paint()
            .into_iter()
            .filter(|(n, _)| *n == child)
            .flat_map(|(_, w)| w)
            .collect();
        assert!(w.iter().any(
            |w| matches!(w, PaintWrite::BorderSideColor { side: 3, color: Some(c) } if c.g == 1.0)
        ));
    }
}
