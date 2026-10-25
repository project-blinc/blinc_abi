//! The one Blinc tree every adapter handle views, and which of its nodes each handle owns.
//!
//! Blinc keeps one queue of property writes and one binding registry, both keyed by node id, and
//! node ids are unique only within a Blinc tree; with one tree they are unique everywhere, so a
//! write or binding can never reach another handle's node. A handle owns the nodes it created,
//! and dropping or disposing it removes them. The HashLink and Caribou adapters share this.

use crate::tree::Tree;
use blinc_layout::binding::unregister_node;
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
