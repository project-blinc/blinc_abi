use super::clip_path::{self, Clip};
use super::color;
use super::filter;
use super::gradient;
use super::paint::{Background, PaintWrite, is_paint_property, paint_writes};
use super::quantity::PaintUnits;
use super::shadow;
use super::text_decoration::{self, LINE_THROUGH, OVERLINE, Style, TextDecoration, UNDERLINE};
use super::transform;
use blinc_core::layer::Gradient;
use blinc_core::layer::ImageFit;
use blinc_core::{ClipLength, ClipPath, Color};

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

fn clip(text: &str) -> Clip {
    clip_path::parse(text, &PaintUnits::default())
        .unwrap()
        .unwrap()
}

fn clip_error(text: &str) -> String {
    clip_path::parse(text, &PaintUnits::default()).unwrap_err()
}

#[test]
fn clip_paths_read_each_shape() {
    use ClipLength::{Percent, Px};
    assert!(
        clip_path::parse("none", &PaintUnits::default())
            .unwrap()
            .is_none()
    );
    match clip("circle(40% at left top)").path {
        ClipPath::Circle { radius, center } => {
            assert_eq!(radius, Some(Percent(40.0)));
            assert_eq!(center, (Percent(0.0), Percent(0.0)));
        }
        other => panic!("{other:?}"),
    }
    // A radius left out, or closest-side, reaches the closest side; the centre is the box's.
    for text in ["circle()", "circle(closest-side)"] {
        match clip(text).path {
            ClipPath::Circle { radius, center } => {
                assert_eq!(radius, None);
                assert_eq!(center, (Percent(50.0), Percent(50.0)));
            }
            other => panic!("{other:?}"),
        }
    }
    match clip("circle(2em at 10px 25%)").path {
        ClipPath::Circle { radius, center } => {
            assert_eq!(radius, Some(Px(32.0)));
            assert_eq!(center, (Px(10.0), Percent(25.0)));
        }
        other => panic!("{other:?}"),
    }
    match clip("ellipse(50% 30% at bottom right)").path {
        ClipPath::Ellipse { rx, ry, center } => {
            assert_eq!((rx, ry), (Some(Percent(50.0)), Some(Percent(30.0))));
            assert_eq!(center, (Percent(100.0), Percent(100.0)));
        }
        other => panic!("{other:?}"),
    }
    match clip("inset(8px 12px round 16px)").path {
        ClipPath::Inset {
            top,
            right,
            bottom,
            left,
            round,
        } => {
            assert_eq!(
                (top, right, bottom, left),
                (Px(8.0), Px(12.0), Px(8.0), Px(12.0))
            );
            assert_eq!(round, Some(16.0));
        }
        other => panic!("{other:?}"),
    }
    match clip("inset(1px 2px 3px)").path {
        ClipPath::Inset { left, .. } => assert_eq!(left, Px(2.0), "left copies right"),
        other => panic!("{other:?}"),
    }
    match clip("rect(0 50% 100% 10px)").path {
        ClipPath::Rect {
            top,
            right,
            bottom,
            left,
            round,
        } => {
            assert_eq!(
                (top, right, bottom, left),
                (Px(0.0), Percent(50.0), Percent(100.0), Px(10.0))
            );
            assert_eq!(round, None);
        }
        other => panic!("{other:?}"),
    }
    match clip("xywh(10px 20px 30% 40% round 4px)").path {
        ClipPath::Xywh { x, y, w, h, round } => {
            assert_eq!(
                (x, y, w, h),
                (Px(10.0), Px(20.0), Percent(30.0), Percent(40.0))
            );
            assert_eq!(round, Some(4.0));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn clip_polygons_and_paths_carry_their_fill_rule() {
    use ClipLength::{Percent, Px};
    let triangle = clip("polygon(50% 0, 100% 100%, 0 100%)");
    assert!(!triangle.even_odd);
    match triangle.path {
        ClipPath::Polygon { points } => assert_eq!(
            points,
            vec![
                (Percent(50.0), Px(0.0)),
                (Percent(100.0), Percent(100.0)),
                (Px(0.0), Percent(100.0)),
            ]
        ),
        other => panic!("{other:?}"),
    }
    assert!(!clip("polygon(nonzero, 0 0, 10px 0, 0 10px)").even_odd);
    let star = clip("polygon(evenodd, 0 0, 10px 0, 0 10px)");
    assert!(star.even_odd);
    assert!(matches!(star.path, ClipPath::Polygon { points } if points.len() == 3));

    match clip("path(\"M 0 0 H 10 V 10 H 0 Z\")").path {
        ClipPath::Path { vertices } => assert_eq!(
            vertices,
            vec![
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0)
            ]
        ),
        other => panic!("{other:?}"),
    }
    // Rings are told apart by a vertex at 1e30; the rule precedes the data.
    let two = clip("path(evenodd, 'M0,0h4v4h-4zM1 1h2v2h-2z')");
    assert!(two.even_odd);
    match two.path {
        ClipPath::Path { vertices } => {
            let breaks: Vec<_> = vertices.iter().filter(|v| v.0 >= 1e29).collect();
            assert_eq!(breaks.len(), 1);
            assert_eq!(vertices.len(), 5 + 1 + 5);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn clip_path_curves_and_arcs_are_cut_into_steps_ending_where_they_say() {
    for data in [
        "M0 0 C 10 0 20 10 20 20 S 0 40 0 20 Z",
        "M0 0 Q 10 0 10 10 T 20 20 Z",
        "M0 10 A 10 10 0 0 1 20 10 A 10 10 0 0 1 0 10 Z",
        "m0 0 l10 0 0 10 -10 0 z",
    ] {
        let ClipPath::Path { vertices } = clip(&format!("path(\"{data}\")")).path else {
            panic!("{data}");
        };
        assert!(vertices.len() > 4, "{data}");
        assert_eq!(vertices.first(), vertices.last(), "closed: {data}");
    }
    // A half circle of radius 10 reaches its far end and bulges by its radius.
    let ClipPath::Path { vertices } = clip("path('M0 0 A 10 10 0 0 1 20 0 Z')").path else {
        panic!();
    };
    assert!(
        vertices
            .iter()
            .any(|v| (v.0 - 20.0).abs() < 1e-3 && v.1.abs() < 1e-3)
    );
    let deepest = vertices.iter().map(|v| v.1.abs()).fold(0.0, f32::max);
    assert!((deepest - 10.0).abs() < 0.1, "{deepest}");
}

#[test]
fn clip_path_values_that_are_not_shapes_say_why() {
    assert!(clip_error("blur(2px)").contains("not circle, ellipse"));
    assert!(clip_error("red").contains("expected none or a shape function"));
    assert!(clip_error("circle(1px 2px)").contains("takes one radius"));
    assert!(clip_error("circle(-4px)").contains("not negative"));
    assert!(clip_error("circle(farthest-side)").contains("farthest-side is not supported"));
    assert!(clip_error("circle(10px at 1px 2px 3px)").contains("one or two values"));
    assert!(clip_error("ellipse(10px)").contains("two radii or none"));
    assert!(clip_error("inset()").contains("one to four lengths"));
    assert!(clip_error("inset(1px 2px 3px 4px 5px)").contains("one to four lengths"));
    assert!(clip_error("inset(10 20)").contains("needs a unit"));
    assert!(clip_error("inset(1px round 10%)").contains("length in pixels"));
    assert!(clip_error("inset(1px round 1px 2px)").contains("one length"));
    assert!(clip_error("rect(1px 2px 3px)").contains("rect() takes four lengths"));
    assert!(clip_error("xywh(1px 2px)").contains("xywh() takes four lengths"));
    assert!(clip_error("polygon()").contains("needs points"));
    assert!(clip_error("polygon(0 0, 10px)").contains("two lengths"));
    assert!(clip_error("path(M0 0)").contains("quoted string"));
    assert!(clip_error("path(\"\")").contains("holds no shape"));
    assert!(clip_error("path(\"L 0 0\")").contains("starts with M"));
    assert!(clip_error("path(\"M 0\")").contains("middle of a command"));
    assert!(clip_error("path(\"M 0 0 A 1 1 0 2 0 5 5\")").contains("arc flag"));
    assert!(clip_error("path(evenodd nonzero, 'M0 0')").contains("fill rule"));
}

#[test]
fn clip_path_is_a_paint_property_and_none_unsets_it() {
    use super::paint::is_paint_property;
    assert!(is_paint_property("clip-path"));
    let units = PaintUnits::default();
    let set = paint_writes("clip-path", Some("inset(4px)"), &units)
        .unwrap()
        .unwrap();
    assert!(matches!(set.as_slice(), [PaintWrite::ClipPath(Some(_))]));
    for unset in [
        paint_writes("clip-path", None, &units),
        paint_writes("clip-path", Some("none"), &units),
    ] {
        assert!(matches!(
            unset.unwrap().unwrap().as_slice(),
            [PaintWrite::ClipPath(None)]
        ));
    }
    assert!(
        paint_writes("clip-path", Some("circle(zz)"), &units)
            .unwrap()
            .is_err()
    );
}

#[test]
fn overflow_fade_takes_a_distance_for_each_side() {
    let units = PaintUnits::default();
    let fade = |v: &str| match paint_writes("overflow-fade", Some(v), &units)
        .unwrap()
        .unwrap()
        .as_slice()
    {
        [PaintWrite::OverflowFade(f)] => *f,
        other => panic!("{other:?}"),
    };
    assert_eq!(fade("12px"), [12.0; 4]);
    assert_eq!(fade("0 20px"), [0.0, 20.0, 0.0, 20.0]);
    assert_eq!(fade("1px 2px 3px"), [1.0, 2.0, 3.0, 2.0]);
    assert_eq!(fade("1px 2px 3px 4px"), [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(fade("2em"), [32.0; 4]);
    assert_eq!(fade("none"), [0.0; 4]);
    for (bad, why) in [
        ("-4px", "nonnegative"),
        ("10%", "percentage"),
        ("1px 2px 3px 4px 5px", "one to four"),
        ("big", "expected a length"),
    ] {
        let e = paint_writes("overflow-fade", Some(bad), &units)
            .unwrap()
            .unwrap_err();
        assert!(e.contains(why), "{bad}: {e}");
    }
    assert!(matches!(
        paint_writes("overflow-fade", None, &units).unwrap().unwrap().as_slice(),
        [PaintWrite::OverflowFade(f)] if *f == [0.0; 4]
    ));
}

#[test]
fn backdrop_filters_read_blur_and_colour_filters() {
    let units = PaintUnits::default();
    let b = filter::parse_backdrop(
        "blur(10px) brightness(1.2) saturate(1.8) hue-rotate(90deg)",
        &units,
    )
    .unwrap()
    .unwrap();
    assert_eq!(b.blur, 10.0);
    assert!(near(b.colors[0], 1.2) && near(b.colors[5], 1.8) && near(b.colors[3], 90.0));
    assert_eq!(
        (b.colors[1], b.colors[2], b.colors[4], b.colors[6]),
        (1.0, 0.0, 0.0, 0.0)
    );
    let only = filter::parse_backdrop("grayscale(1)", &units)
        .unwrap()
        .unwrap();
    assert_eq!(
        (only.blur, only.colors[2]),
        (0.0, 1.0),
        "colour filters alone blur nothing"
    );
    assert!(filter::parse_backdrop("none", &units).unwrap().is_none());
    for (bad, why) in [
        ("opacity(0.5)", "not opacity()"),
        ("drop-shadow(1px 1px)", "not drop-shadow()"),
        ("url(#f)", "not a filter"),
        ("blur(-2px)", "nonnegative"),
    ] {
        let e = filter::parse_backdrop(bad, &units).unwrap_err();
        assert!(e.contains(why), "{bad}: {e}");
    }
}

#[test]
fn a_backdrop_blur_fills_the_node_or_sits_under_its_colour() {
    use super::filter::BACKDROP_IDENTITY;
    let alone = [("backdrop-filter", "blur(8px) invert(1)")];
    let units = PaintUnits {
        declared: &alone,
        ..PaintUnits::default()
    };
    let writes = paint_writes("backdrop-filter", Some("blur(8px) invert(1)"), &units)
        .unwrap()
        .unwrap();
    match writes.as_slice() {
        [
            PaintWrite::BackdropFilter(f),
            PaintWrite::Background(Background::Blur(style)),
        ] => {
            assert_eq!(f[4], 1.0);
            assert_eq!((style.radius, style.tint.is_none()), (8.0, true));
        }
        other => panic!("{other:?}"),
    }
    // A background colour of its own is the blur's tint, whichever declaration is read first.
    let both = [
        ("backdrop-filter", "blur(8px)"),
        ("background", "rgba(255, 255, 255, 0.25)"),
    ];
    let units = PaintUnits {
        declared: &both,
        ..PaintUnits::default()
    };
    let filter_writes = paint_writes("backdrop-filter", Some("blur(8px)"), &units)
        .unwrap()
        .unwrap();
    assert!(matches!(
        filter_writes.as_slice(),
        [PaintWrite::BackdropFilter(_)]
    ));
    let fill = paint_writes("background", Some("rgba(255, 255, 255, 0.25)"), &units)
        .unwrap()
        .unwrap();
    match fill.as_slice() {
        [PaintWrite::Background(Background::Blur(style))] => {
            assert_eq!(style.radius, 8.0);
            assert!(near(style.tint.unwrap().a, 0.25));
        }
        other => panic!("{other:?}"),
    }
    // A gradient is not blurred under; without a backdrop-filter a colour is a colour.
    let gradient = paint_writes("background", Some("linear-gradient(red, blue)"), &units)
        .unwrap()
        .unwrap();
    assert!(matches!(
        gradient.as_slice(),
        [PaintWrite::Background(Background::Gradient(_))]
    ));
    let plain = paint_writes("background", Some("red"), &PaintUnits::default())
        .unwrap()
        .unwrap();
    assert!(matches!(
        plain.as_slice(),
        [PaintWrite::Background(Background::Solid(_))]
    ));
    // Unset, the filters go back and a node with no background of its own is not filled.
    let unset = paint_writes("backdrop-filter", None, &PaintUnits::default())
        .unwrap()
        .unwrap();
    assert!(matches!(
        unset.as_slice(),
        [PaintWrite::BackdropFilter(f), PaintWrite::Background(Background::None)] if *f == BACKDROP_IDENTITY
    ));
    let kept = [("background", "red")];
    let unset = paint_writes(
        "backdrop-filter",
        None,
        &PaintUnits {
            declared: &kept,
            ..PaintUnits::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(unset.as_slice(), [PaintWrite::BackdropFilter(_)]));
    let none = paint_writes("backdrop-filter", Some("none"), &PaintUnits::default())
        .unwrap()
        .unwrap();
    assert!(matches!(
        none.as_slice(),
        [
            PaintWrite::BackdropFilter(_),
            PaintWrite::Background(Background::None)
        ]
    ));
    assert!(
        paint_writes("backdrop-filter", Some("opacity(1)"), &units)
            .unwrap()
            .is_err()
    );
}

fn decoration(name: &str, value: &str, declared: &[(&str, &str)]) -> Option<TextDecoration> {
    let units = PaintUnits {
        declared,
        ..PaintUnits::default()
    };
    match paint_writes(name, Some(value), &units)
        .unwrap()
        .unwrap()
        .as_slice()
    {
        [] => None,
        [PaintWrite::TextDecoration(d)] => *d,
        other => panic!("{other:?}"),
    }
}

#[test]
fn text_decoration_reads_lines_style_colour_and_thickness_in_any_order() {
    let d = decoration("text-decoration", "underline", &[]).unwrap();
    assert_eq!(
        (d.lines, d.style, d.color, d.thickness, d.offset),
        (UNDERLINE, Style::Solid, None, None, None)
    );
    let d = decoration(
        "text-decoration",
        "red dotted 2px line-through underline",
        &[],
    )
    .unwrap();
    assert_eq!(
        (d.lines, d.style, d.thickness),
        (UNDERLINE | LINE_THROUGH, Style::Dotted, Some(2.0))
    );
    assert_eq!(d.color.map(|c| (c.r, c.g, c.b)), Some((1.0, 0.0, 0.0)));
    let d = decoration("text-decoration", "overline double rgb(0 128 0)", &[]).unwrap();
    assert_eq!((d.lines, d.style), (OVERLINE, Style::Double));
    assert!(d.color.is_some_and(|c| (c.g - 128.0 / 255.0).abs() < 1e-3));
    let d = decoration("text-decoration", "underline currentcolor from-font", &[]).unwrap();
    assert_eq!((d.color, d.thickness), (None, None));
    let d = decoration("text-decoration", "10% underline", &[]).unwrap();
    assert_eq!(d.thickness, Some(1.6));
    // A colour or a style alone names no line: none.
    assert_eq!(decoration("text-decoration", "red", &[]).unwrap().lines, 0);
    assert_eq!(decoration("text-decoration", "none", &[]).unwrap().lines, 0);
}

#[test]
fn text_decoration_longhands_stand_over_the_shorthand_and_adjust_the_line() {
    let beside = [
        ("text-decoration", "underline solid"),
        ("text-decoration-color", "blue"),
        ("text-decoration-style", "dashed"),
        ("text-decoration-thickness", "3px"),
        ("text-underline-offset", "4px"),
    ];
    // The shorthand writes the whole of it.
    let d = decoration("text-decoration", "underline solid", &beside).unwrap();
    assert_eq!(
        (d.lines, d.style, d.thickness, d.offset),
        (UNDERLINE, Style::Dashed, Some(3.0), Some(4.0))
    );
    assert!(d.color.is_some_and(|c| c.b == 1.0));
    // With the shorthand there, the line's own declaration writes nothing, and the rest only adjust.
    assert_eq!(
        decoration("text-decoration-line", "overline", &beside),
        None
    );
    for (name, value) in &beside[1..] {
        assert_eq!(decoration(name, value, &beside), None, "{name}");
    }
    // Alone, the line is the decoration, taking the others beside it.
    let longhands = [
        ("text-decoration-line", "overline"),
        ("text-decoration-style", "double"),
        ("text-underline-offset", "0.25em"),
    ];
    let d = decoration("text-decoration-line", "overline", &longhands).unwrap();
    assert_eq!(
        (d.lines, d.style, d.offset),
        (OVERLINE, Style::Double, Some(4.0))
    );
    // A style without a line draws nothing.
    assert_eq!(
        decoration(
            "text-decoration-style",
            "dotted",
            &[("text-decoration-style", "dotted")]
        ),
        None
    );
    // A longhand that cannot be read is left out beside the line, and reported on its own.
    let broken = [
        ("text-decoration-line", "underline"),
        ("text-decoration-style", "wavy"),
    ];
    let d = decoration("text-decoration-line", "underline", &broken).unwrap();
    assert_eq!(d.style, Style::Solid);
}

#[test]
fn text_decoration_that_is_not_understood_says_why() {
    let units = PaintUnits::default();
    let error = |name: &str, value: &str| {
        paint_writes(name, Some(value), &units)
            .unwrap()
            .unwrap_err()
    };
    assert!(error("text-decoration-line", "sideways").contains("expected none, underline"));
    assert!(error("text-decoration-line", "blink").contains("not supported"));
    assert!(error("text-decoration-line", "none underline").contains("stands alone"));
    assert!(error("text-decoration-style", "wavy").contains("wavy is not supported"));
    assert!(error("text-decoration-style", "zigzag").contains("expected solid, double"));
    assert!(error("text-decoration-thickness", "-2px").contains("nonnegative"));
    assert!(error("text-decoration-thickness", "thick").contains("expected a length"));
    assert!(error("text-decoration-color", "notacolour").contains("notacolour"));
    assert!(error("text-underline-offset", "far").contains("expected a length"));
    assert!(error("text-decoration", "underline overline none").contains("stands alone"));
    assert!(error("text-decoration", "dotted dashed").contains("one style"));
    assert!(error("text-decoration", "1px 2px").contains("one thickness"));
    assert!(error("text-decoration", "red blue").contains("one colour"));
    // Unset takes the declaration away, which the properties that only adjust it leave alone.
    for name in ["text-decoration", "text-decoration-line"] {
        assert!(matches!(
            paint_writes(name, None, &units)
                .unwrap()
                .unwrap()
                .as_slice(),
            [PaintWrite::TextDecoration(None)]
        ));
    }
    assert!(
        paint_writes("text-decoration-color", None, &units)
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(text_decoration::is_property("text-underline-offset"));
}

fn image_background(
    name: &str,
    value: &str,
    declared: &[(&str, &str)],
) -> Result<(String, ImageFit), String> {
    let units = PaintUnits {
        declared,
        ..PaintUnits::default()
    };
    match paint_writes(name, Some(value), &units).unwrap()?.as_slice() {
        [PaintWrite::Background(Background::Image { source, fit })] => Ok((source.clone(), *fit)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn url_backgrounds_name_a_source_and_how_it_fills() {
    let fit = |value: &str, declared: &[(&str, &str)]| {
        image_background("background-image", value, declared)
            .unwrap()
            .1
    };
    let (source, f) = image_background("background-image", "url(photo.png)", &[]).unwrap();
    assert_eq!(
        (source.as_str(), f),
        ("photo.png", ImageFit::Tile),
        "its own size, repeated"
    );
    for quoted in ["url(\"a b.png\")", "url('a b.png')", "url( a b.png )"] {
        assert_eq!(
            image_background("background", quoted, &[]).unwrap().0,
            "a b.png",
            "{quoted}"
        );
    }
    let size = |v: &'static str| [("background-size", v)];
    assert_eq!(fit("url(x)", &size("cover")), ImageFit::Cover);
    assert_eq!(fit("url(x)", &size("Contain")), ImageFit::Contain);
    assert_eq!(fit("url(x)", &size("100%  100%")), ImageFit::Fill);
    assert_eq!(fit("url(x)", &size("auto")), ImageFit::Tile);
    // The shorthand carries its own, with a position and a repeat, the slash touching or not.
    assert_eq!(fit("url(x) center / cover no-repeat", &[]), ImageFit::Cover);
    assert_eq!(fit("url(x) center/contain", &[]), ImageFit::Contain);
    assert_eq!(
        fit("url(x) 50% 50% / 100% 100% no-repeat", &[]),
        ImageFit::Fill
    );
    assert_eq!(fit("url(x) repeat", &[]), ImageFit::Tile);
    // Its size and repeat are the shorthand's over the longhands beside it.
    assert_eq!(fit("url(x) / contain", &size("cover")), ImageFit::Contain);
    // A cover or a contain is one image, whatever repeats.
    assert_eq!(
        fit(
            "url(x)",
            &[
                ("background-size", "cover"),
                ("background-repeat", "repeat")
            ]
        ),
        ImageFit::Cover
    );
}

#[test]
fn url_backgrounds_that_cannot_be_drawn_say_why() {
    let error = |value: &str, declared: &[(&str, &str)]| {
        image_background("background-image", value, declared).unwrap_err()
    };
    assert!(error("url()", &[]).contains("names no image"));
    assert!(error("url('')", &[]).contains("names no image"));
    assert!(error("url(x) no-repeat", &[]).contains("background-repeat no-repeat"));
    assert!(error("url(x) repeat-x", &[]).contains("background-repeat repeat-x"));
    assert!(error("url(x)", &[("background-repeat", "space")]).contains("background-repeat space"));
    assert!(
        error("url(x)", &[("background-size", "40px 40px")]).contains("background-size 40px 40px")
    );
    assert!(error("url(x) / 50%", &[]).contains("background-size 50%"));
    assert!(error("url(x) top left", &[]).contains("only the centre"));
    assert!(error("url(x)", &[("background-position", "10px 10px")]).contains("only the centre"));
    // The properties that only adjust an image background write nothing themselves.
    let units = PaintUnits::default();
    for name in [
        "background-size",
        "background-repeat",
        "background-position",
    ] {
        assert!(
            paint_writes(name, Some("cover"), &units)
                .unwrap()
                .unwrap()
                .is_empty()
        );
        assert!(
            paint_writes(name, None, &units)
                .unwrap()
                .unwrap()
                .is_empty()
        );
    }
    // An image is not blurred under, and a colour is not an image.
    let blurred = [("backdrop-filter", "blur(4px)")];
    assert!(matches!(
        paint_writes(
            "background",
            Some("url(x) / cover"),
            &PaintUnits {
                declared: &blurred,
                ..PaintUnits::default()
            }
        )
        .unwrap()
        .unwrap()
        .as_slice(),
        [PaintWrite::Background(Background::Image { .. })]
    ));
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
    assert!(reason("url()").contains("names no image"));
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
        "text-decoration",
    ] {
        assert!(is_paint_property(name), "{name}");
    }
    for name in ["width", "font-size", "cursor", "display", "text-align", "x"] {
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

    #[test]
    fn lengths_take_calc_min_max_and_clamp() {
        let units = Units {
            font_size: 10.0,
            root_font_size: 16.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
        };
        let write = |name: &str, v: &str| -> Result<Vec<(i32, f32)>, String> {
            layout_writes(name, Some(v), &units).unwrap().map(|w| {
                w.iter()
                    .map(|(id, v)| match v {
                        PropValue::Number(n) => (*id, *n),
                        _ => panic!("not a number"),
                    })
                    .collect()
            })
        };
        assert_eq!(
            write("width", "calc(100vw - 2rem)"),
            Ok(vec![(WIDTH, 768.0)])
        );
        assert_eq!(write("height", "calc(1em + 4px)"), Ok(vec![(HEIGHT, 14.0)]));
        assert_eq!(
            write("width", "clamp(100px, 50vw, 300px)"),
            Ok(vec![(WIDTH, 300.0)])
        );
        assert_eq!(
            write("min-width", "min(10px, 2rem)"),
            Ok(vec![(MIN_WIDTH, 10.0)])
        );
        let third = write("width", "calc(100% / 3)").unwrap();
        assert_eq!(third[0].0, WIDTH_PERCENT);
        assert!((third[0].1 - 1.0 / 3.0).abs() < 1e-6);
        // Inside a shorthand, and where only pixels can go.
        let padding = write("padding", "calc(1rem / 2) calc(2px * 3)").unwrap();
        assert_eq!(
            padding,
            [
                (PADDING_TOP, 8.0),
                (PADDING_RIGHT, 6.0),
                (PADDING_BOTTOM, 8.0),
                (PADDING_LEFT, 6.0)
            ]
        );
        // A percentage added to a length is one thing layout cannot say.
        let mixed = write("width", "calc(100% - 32px)").unwrap_err();
        assert!(mixed.contains("cannot be added"), "{mixed}");
        assert!(write("width", "calc(1px + )").is_err());
        let gap = write("gap", "calc(50% + 10%)").unwrap();
        assert_eq!(gap, [(GAP_Y_PERCENT, 0.6), (GAP_X_PERCENT, 0.6)]);
        assert_eq!(
            write("border-width", "calc(1px + 2px)").unwrap(),
            [
                (BORDER_TOP_WIDTH, 3.0),
                (BORDER_RIGHT_WIDTH, 3.0),
                (BORDER_BOTTOM_WIDTH, 3.0),
                (BORDER_LEFT_WIDTH, 3.0),
            ]
        );
        assert_eq!(
            write("border", "calc(1px + 2px) solid red").unwrap(),
            [
                (BORDER_TOP_WIDTH, 3.0),
                (BORDER_RIGHT_WIDTH, 3.0),
                (BORDER_BOTTOM_WIDTH, 3.0),
                (BORDER_LEFT_WIDTH, 3.0),
            ]
        );
        assert!(write("width", "calc(1e308px * 1e308)").is_err());
    }

    #[test]
    fn display_takes_the_inline_forms_laid_out_as_their_block_ones() {
        let word = |v: &str| -> Result<i32, String> {
            layout_writes("display", Some(v), &Units::default())
                .unwrap()
                .map(|w| match w[0].1 {
                    PropValue::Enum(n) => n,
                    _ => panic!("not a keyword"),
                })
        };
        for (name, code) in [
            ("block", 0),
            ("flex", 1),
            ("grid", 2),
            ("none", 3),
            ("inline-block", 4),
            ("inline-flex", 5),
            ("inline-grid", 6),
            ("inline", 7),
        ] {
            assert_eq!(word(name), Ok(code), "{name}");
        }
        assert!(word("table").is_err());
        // Laid out: inline-block as a block, inline-flex as a flex box, inline-grid as a grid.
        let mut style = taffy::Style::default();
        for (code, shown) in [
            (4, taffy::Display::Block),
            (5, taffy::Display::Flex),
            (6, taffy::Display::Grid),
            (7, taffy::Display::Block),
        ] {
            assert!(crate::layout_props::set_i32(&mut style, DISPLAY, code));
            assert_eq!(style.display, shown, "{code}");
        }
    }

    #[test]
    fn position_sticky_is_read_beside_the_other_positions() {
        let word = |v: &str| -> Result<Vec<(i32, i32)>, String> {
            layout_writes("position", Some(v), &Units::default())
                .unwrap()
                .map(|w| {
                    w.iter()
                        .map(|(id, v)| match v {
                            PropValue::Enum(n) => (*id, *n),
                            _ => panic!("not a keyword"),
                        })
                        .collect()
                })
        };
        assert_eq!(word("sticky"), Ok(vec![(POSITION, 2)]));
        assert_eq!(word("relative"), Ok(vec![(POSITION, 0)]));
        assert_eq!(word("absolute"), Ok(vec![(POSITION, 1)]));
        assert!(word("floating").is_err());
    }

    #[test]
    fn z_index_is_an_integer_or_auto() {
        let write = |v: Option<&str>| {
            layout_writes("z-index", v, &Units::default())
                .unwrap()
                .map(|w| {
                    w.iter()
                        .map(|(id, v)| match v {
                            PropValue::Enum(n) => (*id, Some(*n)),
                            PropValue::Unset => (*id, None),
                            _ => panic!("not an integer"),
                        })
                        .collect::<Vec<_>>()
                })
        };
        assert!(is_layout_property("z-index"));
        assert_eq!(write(Some("3")), Ok(vec![(Z_INDEX, Some(3))]));
        assert_eq!(write(Some("-1")), Ok(vec![(Z_INDEX, Some(-1))]));
        assert_eq!(write(Some("auto")), Ok(vec![(Z_INDEX, None)]));
        assert_eq!(write(None), Ok(vec![(Z_INDEX, None)]));
        assert!(write(Some("1.5")).is_err());
        assert!(write(Some("top")).is_err());
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

#[test]
fn line_clamp_is_a_line_count_or_none() {
    for name in ["line-clamp", "-webkit-line-clamp"] {
        assert!(matches!(
            writes(name, "3").as_slice(),
            [PaintWrite::LineClamp(Some(3))]
        ));
        assert!(matches!(
            writes(name, "none").as_slice(),
            [PaintWrite::LineClamp(Some(0))]
        ));
        assert!(matches!(
            paint_writes(name, None, &PaintUnits::default())
                .unwrap()
                .unwrap()
                .as_slice(),
            [PaintWrite::LineClamp(None)]
        ));
        for bad in ["0", "-1", "1.5", "auto"] {
            assert!(
                paint_writes(name, Some(bad), &PaintUnits::default())
                    .unwrap()
                    .is_err(),
                "{bad}"
            );
        }
    }
}

#[test]
fn text_overflow_is_clip_or_ellipsis() {
    assert!(matches!(
        writes("text-overflow", "ellipsis").as_slice(),
        [PaintWrite::TextOverflow(Some(true))]
    ));
    assert!(matches!(
        writes("text-overflow", "Clip").as_slice(),
        [PaintWrite::TextOverflow(Some(false))]
    ));
    assert!(matches!(
        paint_writes("text-overflow", None, &PaintUnits::default())
            .unwrap()
            .unwrap()
            .as_slice(),
        [PaintWrite::TextOverflow(None)]
    ));
    assert!(
        paint_writes("text-overflow", Some("fade"), &PaintUnits::default())
            .unwrap()
            .is_err()
    );
}

#[test]
fn text_shadow_is_a_list_of_offsets_blur_and_colour_or_none() {
    match writes("text-shadow", "1px 2px 3px red, 0 0 #00f").as_slice() {
        [PaintWrite::TextShadow(Some(list))] => {
            assert_eq!(list.len(), 2);
            assert_eq!(
                (list[0].offset_x, list[0].offset_y, list[0].blur),
                (1.0, 2.0, 3.0)
            );
            assert_eq!(bytes(list[0].color), [255, 0, 0, 255]);
            assert_eq!(
                (list[1].blur, bytes(list[1].color)),
                (0.0, [0, 0, 255, 255])
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(writes("text-shadow", "none").as_slice(), [PaintWrite::TextShadow(Some(l))] if l.is_empty())
    );
    assert!(matches!(
        paint_writes("text-shadow", None, &PaintUnits::default())
            .unwrap()
            .unwrap()
            .as_slice(),
        [PaintWrite::TextShadow(None)]
    ));
    for bad in [
        "1px 2px 3px 4px red",
        "inset 1px 1px red",
        "1px",
        "1px 1px -2px red",
    ] {
        assert!(
            paint_writes("text-shadow", Some(bad), &PaintUnits::default())
                .unwrap()
                .is_err(),
            "{bad}"
        );
    }
}
