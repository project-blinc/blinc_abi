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
