//! Operations on the shared tree over plain Rust types: what each adapter's functions do once their
//! arguments are read out of the runtime's values. The HashLink and Caribou adapters call these.

use crate::shared::{clear_children, id};
use crate::tree::Tree;
use blinc_layout::div::GenericFont;
use blinc_layout::tree::{LayoutNodeId, TextMeasureContext};
use taffy::prelude::Style;

/// What a text node is made from.
pub(crate) struct TextSpec {
    pub content: String,
    /// `None` is the system face.
    pub font_name: Option<String>,
    pub font_size: f32,
    pub line_height: f32,
    pub font_weight: i32,
    /// 0 system, 1 monospace, 2 serif, 3 sans-serif.
    pub generic_font: i32,
    /// Bit 0 wrap, bit 1 italic.
    pub flags: i32,
}

/// A text node owned by `owner`; its raw id.
pub(crate) fn create_text_node(tree: &mut Tree, owner: u64, spec: TextSpec) -> u64 {
    let generic_font = spec.generic_font;
    let context = TextMeasureContext {
        content: spec.content,
        font_size: spec.font_size,
        line_height: spec.line_height,
        letter_spacing: 0.0,
        wrap: spec.flags & 1 != 0,
        // No family of its own is the system face: the platform's UI face where it is installed.
        font_name: spec.font_name.or_else(|| {
            if generic_font == 0 {
                crate::text::system_ui()
            } else {
                None
            }
        }),
        generic_font: match generic_font {
            1 => GenericFont::Monospace,
            2 => GenericFont::Serif,
            3 => GenericFont::SansSerif,
            _ => GenericFont::System,
        },
        font_weight: spec.font_weight.clamp(1, 1000) as u16,
        italic: spec.flags & 2 != 0,
    };
    crate::text::ensure_face(&context);
    let node = tree.layout.create_text_node(Style::default(), context);
    tree.owners.insert(node, owner);
    node.to_raw()
}

/// Puts `new` where `old` is among its parent's children; `old` is detached, not deleted. Nothing happens
/// when `old` has no parent.
pub(crate) fn replace_node(tree: &mut Tree, old: u64, new: u64) {
    let (old, new) = (id(old), id(new));
    let Some(&parent) = tree.layout.ancestors(old).first() else {
        return;
    };
    let children = tree
        .layout
        .children(parent)
        .into_iter()
        .map(|child| if child == old { new } else { child })
        .collect();
    tree.layout.replace_children(parent, children);
}

/// Makes `children` `parent`'s children; none clears them.
pub(crate) fn replace_children(tree: &mut Tree, parent: u64, children: &[u64]) {
    if children.is_empty() {
        clear_children(tree, id(parent));
        return;
    }
    let ids = children.iter().map(|&c| id(c)).collect();
    tree.layout.replace_children(id(parent), ids);
}

/// Opts the direct rows of `group` into shared layout and paint; an empty list resets it. False when a node
/// is not `owner`'s or the structure is invalid.
pub(crate) fn set_grid_rows(tree: &mut Tree, owner: u64, group: u64, rows: &[u64]) -> bool {
    if rows.len() >= i16::MAX as usize {
        return false;
    }
    let group = id(group);
    let rows: Vec<_> = rows.iter().map(|&r| id(r)).collect();
    if tree.owners.get(&group) != Some(&owner)
        || rows.iter().any(|r| tree.owners.get(r) != Some(&owner))
    {
        return false;
    }
    tree.layout.set_grid_rows(group, &rows).is_ok()
}

/// `node`'s children, or `None` for a node that is gone.
pub(crate) fn children(tree: &Tree, node: u64) -> Option<Vec<u64>> {
    if !tree.owners.contains_key(&id(node)) {
        return None;
    }
    Some(
        tree.layout
            .children(id(node))
            .iter()
            .map(|c| c.to_raw())
            .collect(),
    )
}

/// `node`'s ancestors, the parent first, or `None` for a node that is gone.
pub(crate) fn ancestors(tree: &Tree, node: u64) -> Option<Vec<u64>> {
    if !tree.owners.contains_key(&id(node)) {
        return None;
    }
    Some(
        tree.layout
            .ancestors(id(node))
            .iter()
            .map(|a| a.to_raw())
            .collect(),
    )
}

/// Makes `children` `parent`'s children in that order. Children it had and does not keep are detached, not
/// deleted, as are `children` from any other parent. A `parent` that is gone is left alone, and ids of nodes
/// that are gone are skipped: a parent's removal can come before the update of what was placed in it.
pub(crate) fn set_children(tree: &mut Tree, parent: u64, children: &[u64]) {
    let parent = id(parent);
    if !tree.owners.contains_key(&parent) {
        return;
    }
    let ids: Vec<LayoutNodeId> = children
        .iter()
        .map(|&c| id(c))
        .filter(|c| tree.owners.contains_key(c))
        .collect();
    // A child still listed under another parent is taken from it first.
    for &child in &ids {
        if let Some(&old) = tree.layout.ancestors(child).first() {
            if old != parent {
                let kept = tree
                    .layout
                    .children(old)
                    .into_iter()
                    .filter(|&c| c != child)
                    .collect();
                tree.layout.replace_children(old, kept);
            }
        }
    }
    tree.layout.replace_children(parent, ids);
}

/// `node`'s box model as layout resolved it, in layout units: padding, border and margin, each top, right,
/// bottom and left. `None` before it has been laid out.
pub(crate) fn box_model(tree: &Tree, node: u64) -> Option<[f32; 12]> {
    let l = tree.layout.get_layout(id(node))?;
    let sides = |r: taffy::Rect<f32>| [r.top, r.right, r.bottom, r.left];
    let mut out = [0.0; 12];
    for (i, v) in [sides(l.padding), sides(l.border), sides(l.margin)]
        .concat()
        .into_iter()
        .enumerate()
    {
        out[i] = v;
    }
    Some(out)
}

/// `node`'s padding as laid out, top, right, bottom, left, then 1 if it paints a box of its own (a
/// background, a border or a shadow) and 0 if not. `None` when it has no layout.
pub(crate) fn box_edges(tree: &Tree, node: u64) -> Option<[f32; 5]> {
    let layout = tree.layout.get_layout(id(node))?;
    let p = layout.padding;
    let painted = tree.props.get(&id(node)).is_some_and(|r| {
        let visible_bg = match &r.background {
            Some(blinc_core::Brush::Solid(c)) => c.a > 0.0,
            Some(_) => true,
            None => false,
        };
        let sides = &r.border_sides;
        let border = r.border_width > 0.0
            || [&sides.top, &sides.right, &sides.bottom, &sides.left]
                .iter()
                .any(|s| s.as_ref().is_some_and(|s| s.width > 0.0));
        visible_bg || border || !r.shadow.is_empty() || !r.inner_shadow.is_empty()
    });
    Some([
        p.top,
        p.right,
        p.bottom,
        p.left,
        if painted { 1.0 } else { 0.0 },
    ])
}

/// `node`'s own text alignment, 0 left, 1 centre, 2 right; -1 when it has none.
pub(crate) fn text_align(tree: &Tree, node: u64) -> i32 {
    match tree.props.get(&id(node)).and_then(|p| p.text_align) {
        Some(blinc_layout::div::TextAlign::Left) => 0,
        Some(blinc_layout::div::TextAlign::Center) => 1,
        Some(blinc_layout::div::TextAlign::Right) => 2,
        None => -1,
    }
}

/// Text limits of this node or its nearest declaring ancestor: the line count (0 for none) and the
/// text-overflow (-1 undeclared, 0 clip, 1 ellipsis).
pub(crate) fn text_limits(tree: &Tree, node: u64) -> (i32, i32) {
    let mut lines = None;
    let mut overflow = None;
    if !tree.line_clamps.is_empty() || !tree.text_overflows.is_empty() {
        for n in std::iter::once(id(node)).chain(tree.layout.ancestors(id(node))) {
            if lines.is_none() {
                lines = tree.line_clamps.get(&n).copied();
            }
            if overflow.is_none() {
                overflow = tree.text_overflows.get(&n).copied();
            }
            if lines.is_some() && overflow.is_some() {
                break;
            }
        }
    }
    (lines.unwrap_or(0) as i32, overflow.map_or(-1, i32::from))
}

/// One walk of a frame: its records and polygon points, and whether its glyphs overflowed the atlases.
pub(crate) struct Attempt {
    records: Vec<f32>,
    points: Vec<f32>,
    atlas_full: bool,
}

/// A frame whose glyphs overflow the atlases starts them afresh, once. The second walk stands even
/// if it overflows too: its records name the glyphs that did fit, so they must stay in the atlases,
/// and the glyphs that did not are left out.
pub(crate) fn build_with_retry<R>(
    mut walk: impl FnMut(&mut R) -> Attempt,
    renderer: &mut R,
    mut clear: impl FnMut(&mut R),
) -> (Vec<f32>, Vec<f32>) {
    let first = walk(renderer);
    if !first.atlas_full {
        return (first.records, first.points);
    }
    clear(renderer);
    let second = walk(renderer);
    (second.records, second.points)
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    fn attempt(tag: f32, atlas_full: bool) -> Attempt {
        Attempt {
            records: vec![tag],
            points: vec![tag + 100.0],
            atlas_full,
        }
    }

    #[test]
    fn a_frame_that_fits_is_walked_once_and_the_atlases_kept() {
        let (mut walks, mut clears) = (0, 0);
        let (records, points) = build_with_retry(
            |_: &mut ()| {
                walks += 1;
                attempt(1.0, false)
            },
            &mut (),
            |_| clears += 1,
        );
        assert_eq!((walks, clears), (1, 0));
        assert_eq!((records, points), (vec![1.0], vec![101.0]));
    }

    #[test]
    fn an_overflowing_frame_is_walked_again_after_the_atlases_start_afresh() {
        let (mut walks, mut clears) = (0, 0);
        let (records, points) = build_with_retry(
            |_: &mut ()| {
                walks += 1;
                attempt(walks as f32, walks == 1)
            },
            &mut (),
            |_| clears += 1,
        );
        assert_eq!((walks, clears), (2, 1));
        assert_eq!((records, points), (vec![2.0], vec![102.0]));
    }

    #[test]
    fn a_frame_that_overflows_twice_keeps_the_atlases_its_records_name() {
        // The atlases are cleared between the walks, never after the last: clearing then
        // would empty what the records refer to, and every glyph would draw blank.
        let (mut walks, mut clears) = (0, 0);
        let (records, points) = build_with_retry(
            |_: &mut ()| {
                walks += 1;
                attempt(walks as f32, true)
            },
            &mut (),
            |_| clears += 1,
        );
        assert_eq!((walks, clears), (2, 1));
        // The second walk's records and its polygon points, not the first's, and not none.
        assert_eq!((records, points), (vec![2.0], vec![102.0]));
    }
}

/// The primitives to draw under `root`, as a display list (see `display_list`): `params` is eight f32s: the
/// device pixels per layout unit text is rasterized for; the theme's corner `n` (0 for no smoothing),
/// smoothing threshold and full radius; and the colour, straight RGBA, of text that neither it nor an
/// ancestor sets. Returns the packed records, polygon points after them, and how many records are to draw.
pub(crate) fn display_list(tree: &mut Tree, root: u64, params: [f32; 8]) -> (Vec<f32>, usize) {
    let display_scale = params[0];
    let shapes = crate::display_list::Shapes {
        n: params[1],
        threshold: params[2],
        radius_full: params[3],
    };
    let text_color = [params[4], params[5], params[6], params[7]];
    let mut renderer = crate::text::renderer();
    let (mut records, points) = build_with_retry(
        |renderer: &mut blinc_text::TextRenderer| {
            let mut glyphs = crate::display_list::Glyphs {
                renderer,
                display_scale: if display_scale > 0.0 {
                    display_scale
                } else {
                    1.0
                },
                atlas_full: false,
                shapes,
                points: Vec::new(),
            };
            let mut records = Vec::new();
            crate::display_list::append(
                tree,
                id(root),
                (0.0, 0.0),
                1.0,
                text_color,
                crate::display_list::IDENTITY,
                &mut Vec::new(),
                &mut glyphs,
                &mut records,
            );
            Attempt {
                records,
                points: std::mem::take(&mut glyphs.points),
                atlas_full: glyphs.atlas_full,
            }
        },
        &mut *renderer,
        |renderer| {
            renderer.clear();
            crate::text::note_atlas_cleared();
        },
    );
    use crate::display_list::{RECORD_FLOATS, SHAPE_POLYGON};
    let drawn = records.len() / RECORD_FLOATS;
    // Polygon offsets count rows from the points' start, which is right after the records.
    let base = (drawn * RECORD_FLOATS / 4) as f32;
    for r in 0..drawn {
        if records[r * RECORD_FLOATS + 94] == SHAPE_POLYGON {
            records[r * RECORD_FLOATS + 96] += base;
        }
    }
    records.extend_from_slice(&points);
    while records.len() % RECORD_FLOATS != 0 {
        records.push(0.0);
    }
    (records, drawn)
}

/// Makes `node` draw the image the caller knows as `slot` in its content box; a negative `slot` stops it.
pub(crate) fn set_image(tree: &mut Tree, node: u64, slot: i32) {
    if slot < 0 {
        tree.images.remove(&id(node));
        tree.image_mappings.remove(&id(node));
    } else {
        tree.images.insert(id(node), slot);
    }
}

/// Makes `node` paint itself in its content box as the canvas known as `slot`; a negative `slot` stops it.
pub(crate) fn set_canvas(tree: &mut Tree, node: u64, slot: i32) {
    tree.set_canvas(id(node), (slot >= 0).then_some(slot));
}

/// Draws `node` as a notch from twelve f32s (see `Tree.setNotch`); `None` draws it as a box again.
pub(crate) fn set_notch(tree: &mut Tree, node: u64, data: Option<[f32; 12]>) {
    match data {
        None => {
            tree.notches.remove(&id(node));
        }
        Some(d) => {
            let mut rows = [[0.0f32; 4]; 3];
            for (i, row) in rows.iter_mut().enumerate() {
                for (j, v) in row.iter_mut().enumerate() {
                    *v = d[i * 4 + j];
                }
            }
            tree.notches.insert(id(node), rows);
        }
    }
}

/// Draws `node` moved by `(dx, dy)` and at size `(w, h)` with its children clipped when `w` is not negative, as
/// a layout animation does; `clear` draws it at its layout again.
pub(crate) fn set_visual(
    tree: &mut Tree,
    node: u64,
    dx: f32,
    dy: f32,
    w: f32,
    h: f32,
    clear: bool,
) {
    if clear {
        tree.visuals.remove(&id(node));
    } else {
        tree.visuals.insert(id(node), [dx, dy, w, h]);
    }
}

/// Makes the hit test pass through `node` and everything inside it, or not.
pub(crate) fn set_pass_through(tree: &mut Tree, node: u64, through: bool) {
    tree.set_pointer_events(id(node), Some(!through));
}

/// Styles an overlay scrollbar: width zero hides it; `track` is straight-alpha 0xAARRGGBB, zero transparent.
pub(crate) fn set_scrollbar_style(tree: &mut Tree, node: u64, width: f32, track: i32) {
    let node = id(node);
    if tree.layout.get_style(node).is_none() || !width.is_finite() || width < 0.0 {
        return;
    }
    if width == 4.0 && track == 0 {
        tree.scrollbar_styles.remove(&node);
    } else {
        let c = |shift: i32| ((track >> shift) & 0xff) as f32 / 255.0;
        tree.scrollbar_styles.insert(
            node,
            crate::tree::ScrollbarStyle {
                width,
                track: [c(16), c(8), c(0), c(24)],
            },
        );
    }
}

/// Scrolls container `node`'s content by `(x, y)` and colours its thumb `0xAARRGGBB` (0 hides it).
pub(crate) fn set_scroll(tree: &mut Tree, node: u64, x: f32, y: f32, thumb: i32) {
    let c = |shift: i32| ((thumb >> shift) & 0xff) as f32 / 255.0;
    tree.scrolls.insert(
        id(node),
        crate::tree::Scroll {
            x,
            y,
            thumb: [c(16), c(8), c(0), c(24)],
        },
    );
}

/// Container `node`'s viewport width and height (inside its border) and its content's width and height;
/// `None` before it is laid out.
pub(crate) fn scroll_extent(tree: &Tree, node: u64) -> Option<[f32; 4]> {
    let layout = tree.layout.get_layout(id(node))?;
    let b = layout.border;
    Some([
        layout.size.width - b.left - b.right,
        layout.size.height - b.top - b.bottom,
        layout.scrollable_overflow_rect.right,
        layout.scrollable_overflow_rect.bottom,
    ])
}

/// `node`'s absolute `x, y, width, height`; `None` for a node not laid out.
pub(crate) fn bounds(tree: &Tree, node: u64) -> Option<[f32; 4]> {
    let b = tree.layout.get_absolute_bounds(id(node))?;
    Some([b.x, b.y, b.width, b.height])
}
