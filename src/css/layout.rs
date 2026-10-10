//! CSS layout declarations as property router writes: what the cascade
//! applies to a `LayoutContext`, through the same ids its setters use.

use super::value::split;
use crate::context::PropValue;

/// Property router ids. A `*_PERCENT` id takes a fraction of the parent, 0 to 1.
pub mod id {
    pub const BORDER_WIDTH: i32 = 2;
    pub const WIDTH: i32 = 10;
    pub const HEIGHT: i32 = 11;
    pub const MIN_WIDTH: i32 = 12;
    pub const MAX_WIDTH: i32 = 13;
    pub const MIN_HEIGHT: i32 = 14;
    pub const MAX_HEIGHT: i32 = 15;
    pub const FLEX_DIRECTION: i32 = 19;
    pub const ALIGN_ITEMS: i32 = 20;
    pub const JUSTIFY_CONTENT: i32 = 21;
    pub const ALIGN_SELF: i32 = 22;
    pub const FLEX_GROW: i32 = 23;
    pub const FLEX_SHRINK: i32 = 24;
    pub const FLEX_WRAP: i32 = 25;
    pub const FLEX_BASIS: i32 = 26;
    pub const DISPLAY: i32 = 27;
    pub const OVERFLOW: i32 = 28;
    pub const POSITION: i32 = 29;
    pub const TOP: i32 = 30;
    pub const RIGHT: i32 = 31;
    pub const BOTTOM: i32 = 32;
    pub const LEFT: i32 = 33;
    pub const PADDING_TOP: i32 = 43;
    pub const PADDING_RIGHT: i32 = 44;
    pub const PADDING_BOTTOM: i32 = 45;
    pub const PADDING_LEFT: i32 = 46;
    pub const MARGIN_TOP: i32 = 47;
    pub const MARGIN_RIGHT: i32 = 48;
    pub const MARGIN_BOTTOM: i32 = 49;
    pub const MARGIN_LEFT: i32 = 50;
    pub const GAP_X: i32 = 51;
    pub const GAP_Y: i32 = 52;
    pub const WIDTH_PERCENT: i32 = 53;
    pub const HEIGHT_PERCENT: i32 = 54;
    pub const MIN_WIDTH_PERCENT: i32 = 55;
    pub const MAX_WIDTH_PERCENT: i32 = 56;
    pub const MIN_HEIGHT_PERCENT: i32 = 57;
    pub const MAX_HEIGHT_PERCENT: i32 = 58;
    pub const FLEX_BASIS_PERCENT: i32 = 59;
    pub const BORDER_TOP_WIDTH: i32 = 60;
    pub const BORDER_RIGHT_WIDTH: i32 = 61;
    pub const BORDER_BOTTOM_WIDTH: i32 = 62;
    pub const BORDER_LEFT_WIDTH: i32 = 63;
    pub const GRID_TEMPLATE_COLUMNS: i32 = 85;
    pub const GRID_TEMPLATE_ROWS: i32 = 86;
    pub const GRID_COLUMN: i32 = 87;
    pub const GRID_ROW: i32 = 88;
    pub const ASPECT_RATIO: i32 = 90;
    pub const OVERFLOW_X: i32 = 99;
    pub const OVERFLOW_Y: i32 = 100;
    pub const ALIGN_CONTENT: i32 = 101;
    pub const JUSTIFY_ITEMS: i32 = 102;
    pub const JUSTIFY_SELF: i32 = 103;
    pub const PADDING_TOP_PERCENT: i32 = 104;
    pub const PADDING_RIGHT_PERCENT: i32 = 105;
    pub const PADDING_BOTTOM_PERCENT: i32 = 106;
    pub const PADDING_LEFT_PERCENT: i32 = 107;
    pub const MARGIN_TOP_PERCENT: i32 = 108;
    pub const MARGIN_RIGHT_PERCENT: i32 = 109;
    pub const MARGIN_BOTTOM_PERCENT: i32 = 110;
    pub const MARGIN_LEFT_PERCENT: i32 = 111;
    pub const TOP_PERCENT: i32 = 112;
    pub const RIGHT_PERCENT: i32 = 113;
    pub const BOTTOM_PERCENT: i32 = 114;
    pub const LEFT_PERCENT: i32 = 115;
    pub const GAP_X_PERCENT: i32 = 116;
    pub const GAP_Y_PERCENT: i32 = 117;
    pub const ORDER: i32 = 118;
}

/// What relative lengths are relative to: the element's font size, the
/// root's, and the viewport, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct Units {
    pub font_size: f64,
    pub root_font_size: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
}

impl Default for Units {
    fn default() -> Self {
        Units {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: 0.0,
            viewport_height: 0.0,
        }
    }
}

/// Router writes for one property.
pub type Writes<'a> = Vec<(i32, PropValue<'a>)>;

/// A length-valued property's pixel id, and its percentage id if it has one.
fn length_ids(name: &str) -> Option<(i32, Option<i32>)> {
    use id::*;
    Some(match name {
        "width" => (WIDTH, Some(WIDTH_PERCENT)),
        "height" => (HEIGHT, Some(HEIGHT_PERCENT)),
        "min-width" => (MIN_WIDTH, Some(MIN_WIDTH_PERCENT)),
        "max-width" => (MAX_WIDTH, Some(MAX_WIDTH_PERCENT)),
        "min-height" => (MIN_HEIGHT, Some(MIN_HEIGHT_PERCENT)),
        "max-height" => (MAX_HEIGHT, Some(MAX_HEIGHT_PERCENT)),
        "flex-basis" => (FLEX_BASIS, Some(FLEX_BASIS_PERCENT)),
        "padding-top" => (PADDING_TOP, Some(PADDING_TOP_PERCENT)),
        "padding-right" => (PADDING_RIGHT, Some(PADDING_RIGHT_PERCENT)),
        "padding-bottom" => (PADDING_BOTTOM, Some(PADDING_BOTTOM_PERCENT)),
        "padding-left" => (PADDING_LEFT, Some(PADDING_LEFT_PERCENT)),
        "margin-top" => (MARGIN_TOP, Some(MARGIN_TOP_PERCENT)),
        "margin-right" => (MARGIN_RIGHT, Some(MARGIN_RIGHT_PERCENT)),
        "margin-bottom" => (MARGIN_BOTTOM, Some(MARGIN_BOTTOM_PERCENT)),
        "margin-left" => (MARGIN_LEFT, Some(MARGIN_LEFT_PERCENT)),
        "top" => (TOP, Some(TOP_PERCENT)),
        "right" => (RIGHT, Some(RIGHT_PERCENT)),
        "bottom" => (BOTTOM, Some(BOTTOM_PERCENT)),
        "left" => (LEFT, Some(LEFT_PERCENT)),
        "column-gap" => (GAP_X, Some(GAP_X_PERCENT)),
        "row-gap" => (GAP_Y, Some(GAP_Y_PERCENT)),
        "border-top-width" => (BORDER_TOP_WIDTH, None),
        "border-right-width" => (BORDER_RIGHT_WIDTH, None),
        "border-bottom-width" => (BORDER_BOTTOM_WIDTH, None),
        "border-left-width" => (BORDER_LEFT_WIDTH, None),
        _ => return None,
    })
}

fn enum_id(name: &str) -> Option<i32> {
    use id::*;
    Some(match name {
        "flex-direction" => FLEX_DIRECTION,
        "flex-wrap" => FLEX_WRAP,
        "display" => DISPLAY,
        "position" => POSITION,
        "overflow" => OVERFLOW,
        "overflow-x" => OVERFLOW_X,
        "overflow-y" => OVERFLOW_Y,
        "align-items" => ALIGN_ITEMS,
        "align-self" => ALIGN_SELF,
        "align-content" => ALIGN_CONTENT,
        "justify-content" => JUSTIFY_CONTENT,
        "justify-items" => JUSTIFY_ITEMS,
        "justify-self" => JUSTIFY_SELF,
        _ => return None,
    })
}

/// A keyword's code for the enum property `id`; -1 is `auto` or `normal`.
fn keyword(id: i32, word: &str) -> Option<i32> {
    use id::*;
    let align = |w: &str| {
        Some(match w {
            "start" | "self-start" => 0,
            "end" | "self-end" => 1,
            "flex-start" => 2,
            "flex-end" => 3,
            "center" => 4,
            "baseline" => 5,
            "stretch" => 6,
            "auto" | "normal" => -1,
            _ => return None,
        })
    };
    let justify = |w: &str| {
        Some(match w {
            "start" | "left" => 0,
            "end" | "right" => 1,
            "flex-start" => 2,
            "flex-end" => 3,
            "center" => 4,
            "stretch" => 5,
            "space-between" => 6,
            "space-evenly" => 7,
            "space-around" => 8,
            "normal" => -1,
            _ => return None,
        })
    };
    match id {
        FLEX_DIRECTION => ["row", "column", "row-reverse", "column-reverse"]
            .iter()
            .position(|k| *k == word)
            .map(|i| i as i32),
        FLEX_WRAP => ["nowrap", "wrap", "wrap-reverse"]
            .iter()
            .position(|k| *k == word)
            .map(|i| i as i32),
        DISPLAY => ["block", "flex", "grid", "none"]
            .iter()
            .position(|k| *k == word)
            .map(|i| i as i32),
        POSITION => match word {
            "static" | "relative" => Some(0),
            "absolute" | "fixed" => Some(1),
            _ => None,
        },
        OVERFLOW | OVERFLOW_X | OVERFLOW_Y => match word {
            "visible" => Some(0),
            "clip" => Some(1),
            "hidden" => Some(2),
            "scroll" | "auto" => Some(3),
            _ => None,
        },
        ALIGN_ITEMS | ALIGN_SELF | JUSTIFY_ITEMS | JUSTIFY_SELF => align(word),
        JUSTIFY_CONTENT | ALIGN_CONTENT => justify(word),
        _ => None,
    }
}

fn text_id(name: &str) -> Option<i32> {
    use id::*;
    Some(match name {
        "grid-template-columns" => GRID_TEMPLATE_COLUMNS,
        "grid-template-rows" => GRID_TEMPLATE_ROWS,
        "grid-column" => GRID_COLUMN,
        "grid-row" => GRID_ROW,
        _ => return None,
    })
}

/// The sides, by index from top, that a border shorthand writes.
fn border_shorthand(name: &str) -> Option<&'static [usize]> {
    Some(match name {
        "border" => &[0, 1, 2, 3],
        "border-top" => &[0],
        "border-right" => &[1],
        "border-bottom" => &[2],
        "border-left" => &[3],
        _ => return None,
    })
}

/// A four-sided shorthand's longhands, top, right, bottom, left.
fn sides(name: &str) -> Option<[&'static str; 4]> {
    Some(match name {
        "padding" => [
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
        "margin" => ["margin-top", "margin-right", "margin-bottom", "margin-left"],
        "inset" => ["top", "right", "bottom", "left"],
        "border-width" => [
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
        _ => return None,
    })
}

/// Whether `name` is a layout property: one `layout_writes` reads.
pub fn is_layout_property(name: &str) -> bool {
    border_shorthand(name).is_some()
        || name == "border-style"
        || length_ids(name).is_some()
        || enum_id(name).is_some()
        || text_id(name).is_some()
        || sides(name).is_some()
        || matches!(
            name,
            "gap" | "flex" | "flex-grow" | "flex-shrink" | "order" | "aspect-ratio"
        )
}

fn number(text: &str, name: &str) -> Result<f64, String> {
    let t = text.trim();
    t.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("invalid value for {name}: {text}"))
}

/// A length's pixel id and pixels, or its percentage id and fraction; `auto` is NaN.
fn length(name: &str, text: &str, units: &Units) -> Result<(i32, f64), String> {
    let (px_id, percent_id) = length_ids(name).expect("a length property");
    if (id::BORDER_TOP_WIDTH..=id::BORDER_LEFT_WIDTH).contains(&px_id) {
        return super::border::width(text, units)
            .map(|w| (px_id, w))
            .map_err(|_| format!("invalid value for {name}: {text}"));
    }
    let t = text.trim().to_ascii_lowercase();
    if t == "auto" {
        return Ok((px_id, f64::NAN));
    }
    let bad = || format!("invalid value for {name}: {text}");
    let (v, unit) = super::value::dimension(&t).ok_or_else(bad)?;
    Ok(match unit.as_str() {
        "" | "px" => (px_id, v),
        "%" => (percent_id.ok_or_else(bad)?, v / 100.0),
        "em" => (px_id, v * units.font_size),
        "rem" => (px_id, v * units.root_font_size),
        "vw" => (px_id, v / 100.0 * units.viewport_width),
        "vh" => (px_id, v / 100.0 * units.viewport_height),
        "vmin" => (
            px_id,
            v / 100.0 * units.viewport_width.min(units.viewport_height),
        ),
        "vmax" => (
            px_id,
            v / 100.0 * units.viewport_width.max(units.viewport_height),
        ),
        _ => return Err(bad()),
    })
}

/// The router writes for one CSS layout declaration, `None` value unsetting
/// what it writes; `None` for a property that is not a layout property. A
/// value it cannot read is an error naming it.
pub fn layout_writes<'a>(
    name: &str,
    value: Option<&'a str>,
    units: &Units,
) -> Option<Result<Writes<'a>, String>> {
    use id::*;
    let num = |id: i32, v: f64| (id, PropValue::Number(v as f32));
    // A border's width is layout, and its colour is paint: this reads the width.
    if let Some(which) = border_shorthand(name) {
        return Some(match value {
            None => Ok(which
                .iter()
                .map(|&i| (BORDER_TOP_WIDTH + i as i32, PropValue::Unset))
                .collect()),
            Some(v) => super::border::parse(v, units, false).map(|b| {
                which
                    .iter()
                    .map(|&i| num(BORDER_TOP_WIDTH + i as i32, b.drawn()))
                    .collect()
            }),
        });
    }
    if name == "border-style" {
        return Some(match value {
            None => Ok((0..4)
                .map(|i| (BORDER_TOP_WIDTH + i, PropValue::Unset))
                .collect()),
            Some(v) => {
                let none = split(v, ' ')
                    .iter()
                    .map(|s| matches!(s.to_ascii_lowercase().as_str(), "none" | "hidden"))
                    .collect();
                // Only whether there is a border: a side with none has no width.
                super::quantity::box_sides(none, name).map(|sides| {
                    sides
                        .iter()
                        .enumerate()
                        .filter(|(_, none)| **none)
                        .map(|(i, _)| num(BORDER_TOP_WIDTH + i as i32, 0.0))
                        .collect()
                })
            }
        });
    }
    if let Some(side) = sides(name) {
        let Some(value) = value else {
            return Some(Ok(side
                .iter()
                .flat_map(|n| layout_writes(n, None, units).unwrap().unwrap())
                .collect()));
        };
        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.is_empty() || parts.len() > 4 {
            return Some(Err(format!("invalid value for {name}: {value}")));
        }
        let t = parts[0];
        let r = parts.get(1).copied().unwrap_or(t);
        let b = parts.get(2).copied().unwrap_or(t);
        let l = parts.get(3).copied().unwrap_or(r);
        let mut out = Vec::new();
        for (n, part) in side.iter().zip([t, r, b, l]) {
            match layout_writes(n, Some(part), units)? {
                Ok(w) => out.extend(w),
                Err(e) => return Some(Err(e)),
            }
        }
        return Some(Ok(out));
    }
    if let Some((px_id, _)) = length_ids(name) {
        return Some(match value {
            // The pixel id's unset also resets the field its percentage id writes.
            None => Ok(vec![(px_id, PropValue::Unset)]),
            Some(v) => length(name, v, units).map(|(id, v)| vec![num(id, v)]),
        });
    }
    if let Some(id) = enum_id(name) {
        return Some(match value {
            None => Ok(vec![(id, PropValue::Unset)]),
            Some(v) => keyword(id, v.trim())
                .map(|code| vec![(id, PropValue::Enum(code))])
                .ok_or_else(|| format!("invalid value for {name}: {v}")),
        });
    }
    if let Some(id) = text_id(name) {
        return Some(Ok(vec![(
            id,
            value.map_or(PropValue::Unset, |v| PropValue::Text(Some(v))),
        )]));
    }
    Some(match (name, value) {
        ("gap", None) => Ok(vec![(GAP_Y, PropValue::Unset), (GAP_X, PropValue::Unset)]),
        ("gap", Some(v)) => {
            let parts: Vec<&str> = v.split_whitespace().collect();
            let row = parts.first().copied().unwrap_or("");
            let column = parts.get(1).copied().unwrap_or(row);
            length("row-gap", row, units).and_then(|r| {
                length("column-gap", column, units).map(|c| vec![num(r.0, r.1), num(c.0, c.1)])
            })
        }
        ("flex-grow" | "flex-shrink", v) => {
            let id = if name == "flex-grow" {
                FLEX_GROW
            } else {
                FLEX_SHRINK
            };
            match v {
                None => Ok(vec![(id, PropValue::Unset)]),
                Some(v) => number(v, name).map(|n| vec![num(id, n)]),
            }
        }
        ("flex", None) => Ok(vec![
            (FLEX_GROW, PropValue::Unset),
            (FLEX_SHRINK, PropValue::Unset),
            (FLEX_BASIS, PropValue::Unset),
        ]),
        ("flex", Some(v)) => {
            // `flex: N` is N 1 0%; `none` is 0 0 auto; `auto` is 1 1 auto.
            let text = v.trim();
            let parts: Vec<&str> = text.split_whitespace().collect();
            let (grow, shrink, basis) = match text {
                "none" => ("0", "0", "auto"),
                "auto" => ("1", "1", "auto"),
                _ => (
                    parts.first().copied().unwrap_or(""),
                    parts.get(1).copied().unwrap_or("1"),
                    parts.get(2).copied().unwrap_or("0%"),
                ),
            };
            number(grow, name).and_then(|g| {
                number(shrink, name).and_then(|s| {
                    length("flex-basis", basis, units)
                        .map(|b| vec![num(FLEX_GROW, g), num(FLEX_SHRINK, s), num(b.0, b.1)])
                })
            })
        }
        ("order", None) => Ok(vec![(ORDER, PropValue::Unset)]),
        ("order", Some(v)) => number(v, name).map(|n| vec![(ORDER, PropValue::Enum(n as i32))]),
        ("aspect-ratio", None) => Ok(vec![(ASPECT_RATIO, PropValue::Unset)]),
        ("aspect-ratio", Some(v)) if v.trim() == "auto" => {
            Ok(vec![(ASPECT_RATIO, PropValue::Unset)])
        }
        ("aspect-ratio", Some(v)) => {
            let (w, h) = v.split_once('/').unwrap_or((v, "1"));
            number(w, name).and_then(|w| number(h, name).map(|h| vec![num(ASPECT_RATIO, w / h)]))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::id::*;
    use super::*;

    fn w(name: &str, value: &str) -> Vec<(i32, PropValue<'static>)> {
        let value: &'static str = Box::leak(value.to_string().into_boxed_str());
        layout_writes(
            name,
            Some(value),
            &Units {
                font_size: 20.0,
                root_font_size: 16.0,
                viewport_width: 1000.0,
                viewport_height: 500.0,
            },
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn lengths_and_units() {
        assert_eq!(w("width", "120px"), [(WIDTH, PropValue::Number(120.0))]);
        assert_eq!(w("width", "50%"), [(WIDTH_PERCENT, PropValue::Number(0.5))]);
        assert!(matches!(w("height", "auto")[0], (HEIGHT, PropValue::Number(v)) if v.is_nan()));
        assert_eq!(w("width", "2em"), [(WIDTH, PropValue::Number(40.0))]);
        assert_eq!(w("width", "2rem"), [(WIDTH, PropValue::Number(32.0))]);
        assert_eq!(w("height", "10vh"), [(HEIGHT, PropValue::Number(50.0))]);
        assert!(
            layout_writes("border-top-width", Some("10%"), &Units::default())
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn shorthands_expand_as_css_does() {
        assert_eq!(
            w("padding", "1px 2px"),
            [
                (PADDING_TOP, PropValue::Number(1.0)),
                (PADDING_RIGHT, PropValue::Number(2.0)),
                (PADDING_BOTTOM, PropValue::Number(1.0)),
                (PADDING_LEFT, PropValue::Number(2.0))
            ]
        );
        assert_eq!(
            w("gap", "4px 8px"),
            [
                (GAP_Y, PropValue::Number(4.0)),
                (GAP_X, PropValue::Number(8.0))
            ]
        );
        assert_eq!(
            w("flex", "2"),
            [
                (FLEX_GROW, PropValue::Number(2.0)),
                (FLEX_SHRINK, PropValue::Number(1.0)),
                (FLEX_BASIS_PERCENT, PropValue::Number(0.0))
            ]
        );
        assert!(matches!(w("flex", "none")[2], (FLEX_BASIS, PropValue::Number(v)) if v.is_nan()));
    }

    #[test]
    fn keywords_text_and_unset() {
        assert_eq!(
            w("flex-direction", "column"),
            [(FLEX_DIRECTION, PropValue::Enum(1))]
        );
        assert_eq!(
            w("justify-content", "space-between"),
            [(JUSTIFY_CONTENT, PropValue::Enum(6))]
        );
        assert_eq!(w("align-self", "auto"), [(ALIGN_SELF, PropValue::Enum(-1))]);
        assert_eq!(
            w("grid-template-columns", "1fr 2fr"),
            [(GRID_TEMPLATE_COLUMNS, PropValue::Text(Some("1fr 2fr")))]
        );
        assert_eq!(
            w("aspect-ratio", "16/9"),
            [(ASPECT_RATIO, PropValue::Number(16.0 / 9.0))]
        );
        assert_eq!(
            layout_writes("margin", None, &Units::default())
                .unwrap()
                .unwrap()
                .len(),
            4
        );
        assert!(layout_writes("color", Some("red"), &Units::default()).is_none());
        assert!(
            layout_writes("display", Some("table"), &Units::default())
                .unwrap()
                .is_err()
        );
    }
}
