//! The one Blinc tree every adapter handle views, and which of its nodes each handle owns.
//!
//! Blinc keeps one queue of property writes and one binding registry, both keyed by node id, and
//! node ids are unique only within a Blinc tree; with one tree they are unique everywhere, so a
//! write or binding can never reach another handle's node. A handle owns the nodes it created,
//! and dropping or disposing it removes them. The HashLink and Caribou adapters share this.

use crate::layout_router::{
    BACKDROP_IDENTITY, TreeWrite, take_pending_backdrop, take_pending_even_odd, take_pending_glass,
    take_pending_text, take_pending_text_decoration, take_pending_text_shadow, take_pending_tree,
};
use crate::tree::Tree;
use blinc_layout::binding::unregister_node;
use blinc_layout::stateful::take_pending_partial_prop_updates;
use blinc_layout::tree::{LayoutNodeId, LayoutTree};
use std::cell::UnsafeCell;
use std::sync::Mutex;

/// The one tree. The runtime calls in from one thread, and no call into it
/// re-enters another, so a plain cell suffices.
struct Shared(UnsafeCell<Option<Tree>>);
unsafe impl Sync for Shared {}
static SHARED: Shared = Shared(UnsafeCell::new(None));
pub(crate) static NEXT_HANDLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn shared() -> &'static mut Tree {
    unsafe { (*SHARED.0.get()).get_or_insert_with(Tree::new) }
}

/// Handles dropped since the last flush, whose nodes are still in the tree.
/// A handle may be dropped by its finalizer, inside a collection, where
/// touching the tree or the binding registry is not safe, so the next flush
/// removes them; disposing a handle removes them at once instead.
pub(crate) static RELEASED_HANDLES: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// Removes every node `handle` created, with its bindings and props.
pub(crate) fn release_handle(tree: &mut Tree, handle: u64) {
    let nodes: Vec<LayoutNodeId> = tree
        .owners
        .iter()
        .filter(|&(_, &o)| o == handle)
        .map(|(&n, _)| n)
        .collect();
    forget(tree, &nodes);
    for node in nodes {
        tree.layout.remove_node(node);
        tree.props.remove(&node);
        tree.images.remove(&node);
        tree.canvases.remove(&node);
        tree.scrolls.remove(&node);
    }
}

pub(crate) fn id(raw: u64) -> LayoutNodeId {
    LayoutNodeId::from_raw(raw)
}

/// `node` and every node below it.
pub(crate) fn subtree(layout: &LayoutTree, node: LayoutNodeId) -> Vec<LayoutNodeId> {
    let mut nodes = vec![node];
    let mut i = 0;
    while i < nodes.len() {
        nodes.extend(layout.children(nodes[i]));
        i += 1;
    }
    nodes
}

/// Drop the property bindings of nodes about to be deleted, so a computed
/// bound only to them can be released, and stop tracking them.
pub(crate) fn forget(tree: &mut Tree, nodes: &[LayoutNodeId]) {
    for node in nodes {
        unregister_node(*node);
        tree.owners.remove(node);
        tree.pass_through.remove(node);
        tree.pointer_auto.remove(node);
        tree.notches.remove(node);
        tree.even_odd.remove(node);
        tree.backdrop_filters.remove(node);
        tree.border_styles.remove(node);
        tree.text_decorations.remove(node);
        tree.text_shadows.remove(node);
        tree.text_overflows.remove(node);
        tree.line_clamps.remove(node);
        tree.clamped.remove(node);
        tree.sticky.remove(node);
        tree.sticky_expressions.remove(node);
        tree.z_index.remove(node);
        tree.image_uses.remove(node);
        tree.image_mappings.remove(node);
        tree.background_mappings.remove(node);
        tree.glass_effects.remove(node);
    }
}

/// Delete every node below `parent`, as `LayoutTree::clear_children` does.
pub(crate) fn clear_children(tree: &mut Tree, parent: LayoutNodeId) {
    for child in tree.layout.children(parent) {
        let nodes = subtree(&tree.layout, child);
        forget(tree, &nodes);
    }
    tree.layout.clear_children(parent);
    tree.pruned = true;
}

/// Applies what has been queued since the last flush to `tree`, and reports whether any of it
/// changed what is drawn: a relayout, or a visual property such as an opacity or a colour.
pub(crate) fn apply_pending(tree: &mut Tree) -> bool {
    if std::mem::take(&mut tree.pruned) {
        let layout = &tree.layout;
        tree.props
            .retain(|node, _| layout.get_style(*node).is_some());
        tree.images
            .retain(|node, _| layout.get_style(*node).is_some());
        tree.canvases
            .retain(|node, _| layout.get_style(*node).is_some());
        tree.scrolls
            .retain(|node, _| layout.get_style(*node).is_some());
        tree.scrollbar_styles
            .retain(|node, _| layout.get_style(*node).is_some());
    }
    let mut needs_layout = false;
    let mut painted = false;
    let mut bordered = Vec::new();
    for update in take_pending_partial_prop_updates() {
        let Some(mut style) = tree.layout.get_style(update.node_id) else {
            continue;
        };
        needs_layout |= update.effects.needs_layout;
        if let Some(write) = update.layout_write {
            write(&mut style);
            tree.layout.set_style(update.node_id, style);
        }
        if let Some(write) = update.render_write {
            write(tree.props.entry(update.node_id).or_default());
            painted = true;
            if update.property == blinc_layout::property::PropertyId::BorderWidth {
                bordered.push(update.node_id);
            }
        }
    }
    // Sticky constraints and text/stacking effects use the existing native tree implementation.
    #[derive(Default)]
    struct StickyUpdate {
        position: Option<bool>,
        insets: [Option<Option<f32>>; 4],
    }
    let mut sticky: std::collections::HashMap<LayoutNodeId, StickyUpdate> =
        std::collections::HashMap::new();
    let writes = take_pending_tree();
    let mut orders = Vec::new();
    // Ordinary relative/absolute boxes keep their existing fast path: only sticky nodes
    // need constraint staging, even when an inset was queued before their position.
    for (node, write) in &writes {
        if let TreeWrite::Position(on) = write
            && (*on || tree.sticky.contains_key(node) || sticky.contains_key(node))
        {
            sticky.entry(*node).or_default().position = Some(*on);
        }
    }
    for (node, write) in writes {
        match write {
            TreeWrite::Order(value) => orders.push((node, value)),
            TreeWrite::BackgroundMapping(mapping) => match mapping {
                Some(mapping) if tree.layout.get_style(node).is_some() => {
                    tree.background_mappings.insert(node, mapping);
                }
                _ => {
                    tree.background_mappings.remove(&node);
                }
            },
            TreeWrite::PointerEvents(enabled) => {
                if tree.layout.get_style(node).is_some() {
                    tree.set_pointer_events(node, enabled);
                }
            }
            TreeWrite::Position(_) => {}
            TreeWrite::Inset(i, v) => {
                if sticky.contains_key(&node) || tree.sticky.contains_key(&node) {
                    sticky.entry(node).or_default().insets[i] = Some(v);
                }
            }
            TreeWrite::ZIndex(v) => {
                match v {
                    Some(v) => tree.z_index.insert(node, v),
                    None => tree.z_index.remove(&node),
                };
            }
            TreeWrite::TextOverflow(v) => {
                match v {
                    Some(v) => tree.text_overflows.insert(node, v),
                    None => tree.text_overflows.remove(&node),
                };
            }
            TreeWrite::LineClamp(v) => {
                match v {
                    Some(v) => tree.line_clamps.insert(node, v),
                    None => tree.line_clamps.remove(&node),
                };
                needs_layout = true;
            }
        }
        painted = true;
    }
    needs_layout |= tree.layout.set_orders(orders);
    for (node, update) in sticky {
        if let Some(mut style) = tree.layout.get_style(node) {
            tree.sync_sticky(node, &mut style, update.position, update.insets);
            tree.layout.set_style(node, style);
            needs_layout = true;
        }
    }
    // A border takes layout space: each side's width, as painted, insets the box's content.
    for node in bordered {
        let (Some(props), Some(mut style)) = (tree.props.get(&node), tree.layout.get_style(node))
        else {
            continue;
        };
        let [top, right, bottom, left] =
            crate::display_list::border_sides(props).map(taffy::prelude::LengthPercentage::length);
        let border = taffy::Rect {
            left,
            right,
            top,
            bottom,
        };
        if style.border != border {
            style.border = border;
            tree.layout.set_style(node, style);
            needs_layout = true;
        }
    }
    // Recorded by the render writes above, so applied after them.
    for (node, filter, value) in take_pending_backdrop() {
        let filters = tree
            .backdrop_filters
            .entry(node)
            .or_insert(BACKDROP_IDENTITY);
        filters[filter] = value;
        if *filters == BACKDROP_IDENTITY {
            tree.backdrop_filters.remove(&node);
        }
        painted = true;
    }
    for (node, shadows) in take_pending_text_shadow() {
        match shadows {
            Some(list) => tree.text_shadows.insert(node, list),
            None => tree.text_shadows.remove(&node),
        };
        painted = true;
    }
    for (node, decoration) in take_pending_text_decoration() {
        match decoration {
            Some(d) => tree.text_decorations.insert(node, d),
            None => tree.text_decorations.remove(&node),
        };
        painted = true;
    }
    for (node, even_odd) in take_pending_even_odd() {
        if even_odd {
            tree.even_odd.insert(node);
        } else {
            tree.even_odd.remove(&node);
        }
        painted = true;
    }
    for (node, effects) in take_pending_glass() {
        if let Some(effects) = effects {
            tree.glass_effects.insert(node, effects);
        } else {
            tree.glass_effects.remove(&node);
        }
        painted = true;
    }
    for (node, write) in take_pending_text() {
        needs_layout |= tree.layout.update_text(node, |c| {
            write(c);
            // A new weight, style or font is loaded before layout measures with it.
            crate::text::ensure_face(c);
        });
    }
    needs_layout || painted
}
