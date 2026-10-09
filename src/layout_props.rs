//! Taffy style writes by router property id, shared by the HashLink router
//! (`layout_router`) and owned layout contexts (`LayoutContext::apply`).
//!
//! Ids are ashui's `PropertyId` numbering: Blinc's `PropertyId` up to 42,
//! then ashui's own. Lengths are pixels, and NaN is `auto` for sizes,
//! margins and insets. A `*Percent` id takes a fraction of the parent (0 to
//! 1). Enum codes are those of `ashui.types.Style`; a code out of range
//! writes the property's default.

use blinc_layout::element::BorderSide;
use blinc_layout::property::PropertyId;
use taffy::prelude::*;
use taffy::{Overflow, Point};

pub const PADDING_TOP: i32 = 43;
pub const MARGIN_TOP: i32 = 47;
pub const GAP_X: i32 = 51;
pub const GAP_Y: i32 = 52;
pub const WIDTH_PERCENT: i32 = 53;
pub const FLEX_BASIS_PERCENT: i32 = 59;
pub const GRID_TEMPLATE_COLUMNS: i32 = 85;
pub const GRID_TEMPLATE_ROWS: i32 = 86;
pub const GRID_COLUMN: i32 = 87;
pub const GRID_ROW: i32 = 88;
pub const ASPECT_RATIO: i32 = 90;
pub const OVERFLOW_X: i32 = 99;
pub const OVERFLOW_Y: i32 = 100;
/// Takes `JustifyContent`'s codes.
pub const ALIGN_CONTENT: i32 = 101;
/// Take `AlignItems`'s codes.
pub const JUSTIFY_ITEMS: i32 = 102;
pub const JUSTIFY_SELF: i32 = 103;
/// Top, right, bottom, left, each a fraction of the parent's width as CSS resolves them.
pub const PADDING_TOP_PERCENT: i32 = 104;
pub const MARGIN_TOP_PERCENT: i32 = 108;
/// Top, right, bottom, left insets as fractions of the containing block.
pub const TOP_PERCENT: i32 = 112;
pub const GAP_X_PERCENT: i32 = 116;
pub const GAP_Y_PERCENT: i32 = 117;

/// The Blinc property a layout write is queued and bound under; `None` when
/// `raw` is not a number-valued layout property.
pub fn f32_property(raw: i32) -> Option<PropertyId> {
    use PropertyId as P;
    Some(match raw {
        10 | 53 | ASPECT_RATIO => P::Width,
        11 | 54 => P::Height,
        12 | 55 => P::MinWidth,
        13 | 56 => P::MaxWidth,
        14 | 57 => P::MinHeight,
        15 | 58 => P::MaxHeight,
        16 | 43..=46 | 104..=107 => P::Padding,
        17 | 47..=50 | 108..=111 => P::Margin,
        18 | GAP_X | GAP_Y | GAP_X_PERCENT | GAP_Y_PERCENT => P::Gap,
        23 => P::FlexGrow,
        24 => P::FlexShrink,
        26 | FLEX_BASIS_PERCENT => P::FlexBasis,
        30 | 112 => P::Top,
        31 | 113 => P::Right,
        32 | 114 => P::Bottom,
        33 | 115 => P::Left,
        _ => return None,
    })
}

/// As `f32_property`, for enum-valued layout properties.
pub fn i32_property(raw: i32) -> Option<PropertyId> {
    use PropertyId as P;
    Some(match raw {
        19 => P::FlexDirection,
        20 | JUSTIFY_ITEMS => P::AlignItems,
        21 | ALIGN_CONTENT => P::JustifyContent,
        22 | JUSTIFY_SELF => P::AlignSelf,
        25 => P::FlexWrap,
        27 => P::Display,
        28 | OVERFLOW_X | OVERFLOW_Y => P::Overflow,
        29 => P::Position,
        _ => return None,
    })
}

/// As `f32_property`, for the grid properties written as CSS text.
/// Changing tracks or placement relayouts, as `Display` does.
pub fn string_property(raw: i32) -> Option<PropertyId> {
    (GRID_TEMPLATE_COLUMNS..=GRID_ROW)
        .contains(&raw)
        .then_some(PropertyId::Display)
}

/// The property `unset` resets `raw` under, for every id the setters write.
pub fn unset_property(raw: i32) -> Option<PropertyId> {
    f32_property(raw)
        .or_else(|| i32_property(raw))
        .or_else(|| string_property(raw))
}

/// A border side, made with its width and colour unset, so a side can set
/// one and take the border's other: a negative width and a NaN red, which
/// `display_list` reads as the border's.
pub fn border_side(slot: &mut Option<BorderSide>) -> &mut BorderSide {
    slot.get_or_insert(BorderSide {
        width: -1.0,
        color: blinc_core::Color {
            r: f32::NAN,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
    })
}

/// A length in pixels; NaN is `auto`.
fn dimension(v: f32) -> Dimension {
    if v.is_nan() {
        Dimension::auto()
    } else {
        Dimension::length(v)
    }
}

fn length_auto(v: f32) -> LengthPercentageAuto {
    if v.is_nan() {
        LengthPercentageAuto::auto()
    } else {
        LengthPercentageAuto::length(v)
    }
}

fn side<T>(r: &mut Rect<T>, i: i32) -> &mut T {
    match i {
        0 => &mut r.top,
        1 => &mut r.right,
        2 => &mut r.bottom,
        _ => &mut r.left,
    }
}

/// Writes number property `raw`; false, with `s` unchanged, when it is not one.
pub fn set_f32(s: &mut Style, raw: i32, v: f32) -> bool {
    let px = LengthPercentage::length;
    let pc = LengthPercentage::percent;
    match raw {
        10 => s.size.width = dimension(v),
        11 => s.size.height = dimension(v),
        12 => s.min_size.width = length_auto(v),
        13 => s.max_size.width = length_auto(v),
        14 => s.min_size.height = length_auto(v),
        15 => s.max_size.height = length_auto(v),
        16 => {
            s.padding = Rect {
                left: px(v),
                right: px(v),
                top: px(v),
                bottom: px(v),
            }
        }
        17 => {
            let l = length_auto(v);
            s.margin = Rect {
                left: l,
                right: l,
                top: l,
                bottom: l,
            };
        }
        18 => {
            s.gap = Size {
                width: px(v),
                height: px(v),
            }
        }
        23 => s.flex_grow = v,
        24 => s.flex_shrink = v,
        26 => s.flex_basis = dimension(v),
        30 => s.inset.top = length_auto(v),
        31 => s.inset.right = length_auto(v),
        32 => s.inset.bottom = length_auto(v),
        33 => s.inset.left = length_auto(v),
        43..=46 => *side(&mut s.padding, raw - PADDING_TOP) = px(v),
        47..=50 => *side(&mut s.margin, raw - MARGIN_TOP) = length_auto(v),
        // Taffy's gap.width is the gap between columns, gap.height between rows.
        GAP_X => s.gap.width = px(v),
        GAP_Y => s.gap.height = px(v),
        53 => s.size.width = Dimension::percent(v),
        54 => s.size.height = Dimension::percent(v),
        55 => s.min_size.width = LengthPercentageAuto::percent(v),
        56 => s.max_size.width = LengthPercentageAuto::percent(v),
        57 => s.min_size.height = LengthPercentageAuto::percent(v),
        58 => s.max_size.height = LengthPercentageAuto::percent(v),
        FLEX_BASIS_PERCENT => s.flex_basis = Dimension::percent(v),
        // Width over height; NaN or none positive is no ratio.
        ASPECT_RATIO => s.aspect_ratio = (v.is_finite() && v > 0.0).then_some(v),
        104..=107 => *side(&mut s.padding, raw - PADDING_TOP_PERCENT) = pc(v),
        108..=111 => {
            *side(&mut s.margin, raw - MARGIN_TOP_PERCENT) = LengthPercentageAuto::percent(v)
        }
        112..=115 => *side(&mut s.inset, raw - TOP_PERCENT) = LengthPercentageAuto::percent(v),
        GAP_X_PERCENT => s.gap.width = pc(v),
        GAP_Y_PERCENT => s.gap.height = pc(v),
        _ => return false,
    }
    true
}

fn display(v: i32) -> Display {
    match v {
        0 => Display::Block,
        2 => Display::Grid,
        3 => Display::None,
        _ => Display::Flex,
    }
}

fn flex_direction(v: i32) -> FlexDirection {
    match v {
        1 => FlexDirection::Column,
        2 => FlexDirection::RowReverse,
        3 => FlexDirection::ColumnReverse,
        _ => FlexDirection::Row,
    }
}

fn flex_wrap(v: i32) -> FlexWrap {
    match v {
        1 => FlexWrap::Wrap,
        2 => FlexWrap::WrapReverse,
        _ => FlexWrap::NoWrap,
    }
}

/// `None` is `auto`.
fn align_items(v: i32) -> Option<AlignItems> {
    Some(match v {
        0 => AlignItems::START,
        1 => AlignItems::END,
        2 => AlignItems::FLEX_START,
        3 => AlignItems::FLEX_END,
        4 => AlignItems::CENTER,
        5 => AlignItems::BASELINE,
        6 => AlignItems::STRETCH,
        _ => return None,
    })
}

/// `None` is `normal`.
fn justify_content(v: i32) -> Option<JustifyContent> {
    Some(match v {
        0 => JustifyContent::START,
        1 => JustifyContent::END,
        2 => JustifyContent::FLEX_START,
        3 => JustifyContent::FLEX_END,
        4 => JustifyContent::CENTER,
        5 => JustifyContent::STRETCH,
        6 => JustifyContent::SPACE_BETWEEN,
        7 => JustifyContent::SPACE_EVENLY,
        8 => JustifyContent::SPACE_AROUND,
        _ => return None,
    })
}

fn position(v: i32) -> Position {
    match v {
        1 => Position::Absolute,
        _ => Position::Relative,
    }
}

fn overflow(v: i32) -> Overflow {
    match v {
        1 => Overflow::Clip,
        2 => Overflow::Hidden,
        3 => Overflow::Scroll,
        _ => Overflow::Visible,
    }
}

/// Writes enum property `raw`; false, with `s` unchanged, when it is not one.
pub fn set_i32(s: &mut Style, raw: i32, v: i32) -> bool {
    match raw {
        19 => s.flex_direction = flex_direction(v),
        20 => s.align_items = align_items(v),
        21 => s.justify_content = justify_content(v),
        22 => s.align_self = align_items(v),
        25 => s.flex_wrap = flex_wrap(v),
        27 => s.display = display(v),
        28 => {
            let o = overflow(v);
            s.overflow = Point { x: o, y: o };
        }
        29 => s.position = position(v),
        OVERFLOW_X => s.overflow.x = overflow(v),
        OVERFLOW_Y => s.overflow.y = overflow(v),
        ALIGN_CONTENT => s.align_content = justify_content(v),
        JUSTIFY_ITEMS => s.justify_items = align_items(v),
        JUSTIFY_SELF => s.justify_self = align_items(v),
        _ => return false,
    }
    true
}

/// Writes grid property `raw` from CSS text; text that does not parse, or
/// none, writes the default. False when `raw` is not a grid property.
pub fn set_string(s: &mut Style, raw: i32, v: Option<&str>) -> bool {
    match raw {
        GRID_TEMPLATE_COLUMNS => {
            s.grid_template_columns = v.and_then(crate::grid::template).unwrap_or_default()
        }
        GRID_TEMPLATE_ROWS => {
            s.grid_template_rows = v.and_then(crate::grid::template).unwrap_or_default()
        }
        GRID_COLUMN => {
            s.grid_column = v
                .and_then(crate::grid::line_pair)
                .unwrap_or_else(|| <Style>::DEFAULT.grid_column.clone())
        }
        GRID_ROW => {
            s.grid_row = v
                .and_then(crate::grid::line_pair)
                .unwrap_or_else(|| <Style>::DEFAULT.grid_row.clone())
        }
        _ => return false,
    }
    true
}

/// Puts the field `raw` writes back to a new node's; a percentage id resets
/// the same field as its length id. False when `raw` writes no style field.
pub fn unset(s: &mut Style, raw: i32) -> bool {
    let d = <Style>::DEFAULT;
    match raw {
        10 | 53 => s.size.width = d.size.width,
        11 | 54 => s.size.height = d.size.height,
        12 | 55 => s.min_size.width = d.min_size.width,
        13 | 56 => s.max_size.width = d.max_size.width,
        14 | 57 => s.min_size.height = d.min_size.height,
        15 | 58 => s.max_size.height = d.max_size.height,
        16 => s.padding = d.padding,
        17 => s.margin = d.margin,
        18 => s.gap = d.gap,
        19 => s.flex_direction = d.flex_direction,
        20 => s.align_items = d.align_items,
        21 => s.justify_content = d.justify_content,
        22 => s.align_self = d.align_self,
        23 => s.flex_grow = d.flex_grow,
        24 => s.flex_shrink = d.flex_shrink,
        25 => s.flex_wrap = d.flex_wrap,
        26 | FLEX_BASIS_PERCENT => s.flex_basis = d.flex_basis,
        27 => s.display = d.display,
        28 => s.overflow = d.overflow,
        29 => s.position = d.position,
        30 | 112 => s.inset.top = d.inset.top,
        31 | 113 => s.inset.right = d.inset.right,
        32 | 114 => s.inset.bottom = d.inset.bottom,
        33 | 115 => s.inset.left = d.inset.left,
        43..=46 => *side(&mut s.padding, raw - PADDING_TOP) = d.padding.top,
        104..=107 => *side(&mut s.padding, raw - PADDING_TOP_PERCENT) = d.padding.top,
        47..=50 => *side(&mut s.margin, raw - MARGIN_TOP) = d.margin.top,
        108..=111 => *side(&mut s.margin, raw - MARGIN_TOP_PERCENT) = d.margin.top,
        GAP_X | GAP_X_PERCENT => s.gap.width = d.gap.width,
        GAP_Y | GAP_Y_PERCENT => s.gap.height = d.gap.height,
        GRID_TEMPLATE_COLUMNS => s.grid_template_columns = Vec::new(),
        GRID_TEMPLATE_ROWS => s.grid_template_rows = Vec::new(),
        GRID_COLUMN => s.grid_column = d.grid_column.clone(),
        GRID_ROW => s.grid_row = d.grid_row.clone(),
        ASPECT_RATIO => s.aspect_ratio = None,
        OVERFLOW_X => s.overflow.x = d.overflow.x,
        OVERFLOW_Y => s.overflow.y = d.overflow.y,
        ALIGN_CONTENT => s.align_content = d.align_content,
        JUSTIFY_ITEMS => s.justify_items = d.justify_items,
        JUSTIFY_SELF => s.justify_self = d.justify_self,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setter_id_has_a_property_and_resets() {
        for raw in 0..200 {
            let mut s = Style::default();
            let f = set_f32(&mut s, raw, 7.0);
            let i = set_i32(&mut s, raw, 1);
            let t = set_string(&mut s, raw, Some("1 / 3"));
            assert_eq!(f, f32_property(raw).is_some(), "f32 {raw}");
            assert_eq!(i, i32_property(raw).is_some(), "i32 {raw}");
            assert_eq!(t, string_property(raw).is_some(), "string {raw}");
            assert_eq!(unset(&mut s, raw), f || i || t, "unset {raw}");
            if f || i || t {
                assert_eq!(s, Style::default(), "unset restores {raw}");
            }
        }
    }

    #[test]
    fn sides_percentages_and_axes() {
        let mut s = Style::default();
        set_f32(&mut s, PADDING_TOP + 3, 5.0);
        set_f32(&mut s, MARGIN_TOP_PERCENT + 1, 0.25);
        set_f32(&mut s, TOP_PERCENT + 2, 0.5);
        set_f32(&mut s, MARGIN_TOP, f32::NAN);
        set_i32(&mut s, OVERFLOW_Y, 3);
        assert_eq!(s.padding.left, LengthPercentage::length(5.0));
        assert_eq!(s.margin.right, LengthPercentageAuto::percent(0.25));
        assert_eq!(s.inset.bottom, LengthPercentageAuto::percent(0.5));
        assert_eq!(s.margin.top, LengthPercentageAuto::auto());
        assert_eq!(
            s.overflow,
            Point {
                x: Overflow::Visible,
                y: Overflow::Scroll
            }
        );
    }
}
