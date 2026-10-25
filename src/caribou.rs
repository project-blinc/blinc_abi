//! The layout tree as a Caribou plugin, beside the HashLink and Node adapters: the same shared
//! tree and node ownership (see `shared`), reached from any Caribou language.
//!
//! A `Tree` value is one handle: the nodes it creates are its own, and dropping or disposing it
//! removes them. Node ids cross as `i64` (a node's raw id), and results that HashLink writes into
//! a caller's buffer are written into a `BufferMut` the same way.

use crate::ops::{self, TextSpec};
use crate::shared::{
    NEXT_HANDLE, RELEASED_HANDLES, clear_children as clear, forget, id, release_handle, shared,
    subtree,
};
use crate::types::Value;
use blinc_core::{Brush, Color, CornerRadius};
use caribou_abi::{Buffer, BufferMut, host::Text};
use std::sync::atomic::Ordering;
use taffy::prelude::{AvailableSpace, Size, Style};

/// The 64-bit node ids packed in `buffer`, little-endian, eight bytes each.
fn ids_of(buffer: Buffer) -> Vec<u64> {
    let bytes = unsafe { buffer.as_slice() };
    bytes
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

/// Writes as many of `ids` as fit into `out`, eight bytes each; how many there are.
fn put_ids(ids: &[u64], out: BufferMut) -> i32 {
    let room = out.len() / 8;
    let at = out.as_mut_ptr();
    if !at.is_null() {
        for (i, id) in ids.iter().take(room).enumerate() {
            unsafe { (at as *mut u64).add(i).write_unaligned(*id) };
        }
    }
    ids.len() as i32
}

/// Writes `values` into `out` as f32s; false when `out` is too small.
fn put_floats(values: &[f32], out: BufferMut) -> bool {
    if out.len() < values.len() * 4 {
        return false;
    }
    let at = out.as_mut_ptr() as *mut f32;
    for (i, v) in values.iter().enumerate() {
        unsafe { at.add(i).write_unaligned(*v) };
    }
    true
}

/// A text parameter's string, or `None` for none or the empty text.
fn opt_text(text: Text) -> Option<String> {
    (!text.is_null() && !text.as_str().is_empty()).then(|| text.as_str().to_string())
}

/// One handle onto the process's tree, owning the nodes it created.
pub struct Tree {
    id: u64,
    /// Its nodes are already gone: disposed, so its drop queues nothing.
    released: bool,
}

impl Drop for Tree {
    fn drop(&mut self) {
        if !self.released {
            RELEASED_HANDLES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(self.id);
        }
    }
}

impl Tree {
    pub extern "C" fn new() -> Box<Tree> {
        // The renderer's own registry, so layout measures with the faces text is drawn with.
        crate::text::initialize();
        shared();
        Box::new(Tree {
            id: NEXT_HANDLE.fetch_add(1, Ordering::Relaxed),
            released: false,
        })
    }

    /// A new node with no parent and the default style; its raw id, or 0 once disposed.
    pub extern "C" fn create_node(this: &Tree) -> i64 {
        if this.released {
            return 0;
        }
        let tree = shared();
        let node = tree.layout.create_node(Style::default());
        tree.owners.insert(node, this.id);
        node.to_raw() as i64
    }

    pub extern "C" fn add_child(this: &Tree, parent: i64, child: i64) {
        if !this.released {
            shared()
                .layout
                .add_child(id(parent as u64), id(child as u64));
        }
    }

    pub extern "C" fn remove_node(this: &Tree, node: i64) {
        if this.released {
            return;
        }
        let tree = shared();
        forget(tree, &[id(node as u64)]);
        tree.layout.remove_node(id(node as u64));
        tree.props.remove(&id(node as u64));
    }

    pub extern "C" fn remove_subtree(this: &Tree, node: i64) {
        if this.released {
            return;
        }
        let tree = shared();
        let nodes = subtree(&tree.layout, id(node as u64));
        forget(tree, &nodes);
        tree.layout.remove_subtree(id(node as u64));
        tree.pruned = true;
    }

    pub extern "C" fn clear_children(this: &Tree, parent: i64) {
        if !this.released {
            clear(shared(), id(parent as u64));
        }
    }

    pub extern "C" fn compute_layout(this: &Tree, root: i64, width: f32, height: f32) {
        if this.released {
            return;
        }
        let tree = shared();
        let space = Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        };
        tree.sync_line_clamps();
        tree.layout.compute_layout(id(root as u64), space);
    }

    /// Writes the node's absolute `x, y, width, height` as four f32s into `out`. False, with `out`
    /// untouched, for a node not laid out or a buffer under sixteen bytes.
    pub extern "C" fn get_bounds(this: &Tree, node: i64, out: BufferMut) -> bool {
        if this.released || out.len() < 16 {
            return false;
        }
        let Some(b) = shared().layout.get_absolute_bounds(id(node as u64)) else {
            return false;
        };
        let out = out.as_mut_ptr() as *mut f32;
        for (i, v) in [b.x, b.y, b.width, b.height].into_iter().enumerate() {
            unsafe { out.add(i).write_unaligned(v) };
        }
        true
    }

    /// A text node of `content` in the named font (none for the system face), `flags` bit 0 wrapping and bit 1 italic.
    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn create_text_node(
        this: &Tree,
        content: Text,
        font_name: Text,
        font_size: f32,
        line_height: f32,
        font_weight: i32,
        generic_font: i32,
        flags: i32,
    ) -> i64 {
        if this.released {
            return 0;
        }
        let spec = TextSpec {
            content: content.as_str().to_string(),
            font_name: opt_text(font_name),
            font_size,
            line_height,
            font_weight,
            generic_font,
            flags,
        };
        ops::create_text_node(shared(), this.id, spec) as i64
    }

    /// Puts `new` where `old` is among its parent's children; `old` is detached, not deleted.
    pub extern "C" fn replace_node(this: &Tree, old: i64, new: i64) {
        if !this.released {
            ops::replace_node(shared(), old as u64, new as u64);
        }
    }

    /// Makes the ids in `children` (eight bytes each) `parent`'s children; none clears them.
    pub extern "C" fn replace_children(this: &Tree, parent: i64, children: Buffer) {
        if !this.released {
            ops::replace_children(shared(), parent as u64, &ids_of(children));
        }
    }

    /// As `replace_children`, taking children from other parents and skipping nodes that are gone.
    pub extern "C" fn set_children(this: &Tree, parent: i64, children: Buffer) {
        if !this.released {
            ops::set_children(shared(), parent as u64, &ids_of(children));
        }
    }

    pub extern "C" fn set_grid_rows(this: &Tree, group: i64, rows: Buffer) -> bool {
        !this.released && ops::set_grid_rows(shared(), this.id, group as u64, &ids_of(rows))
    }

    /// Writes `node`'s children into `out` as ids, as many as fit; how many there are (0 for a node that is gone).
    pub extern "C" fn children(this: &Tree, node: i64, out: BufferMut) -> i32 {
        if this.released {
            return 0;
        }
        ops::children(shared(), node as u64).map_or(0, |ids| put_ids(&ids, out))
    }

    /// Writes `node`'s ancestors into `out`, the parent first; how many there are.
    pub extern "C" fn ancestors(this: &Tree, node: i64, out: BufferMut) -> i32 {
        if this.released {
            return 0;
        }
        ops::ancestors(shared(), node as u64).map_or(0, |ids| put_ids(&ids, out))
    }

    /// Writes `node`'s padding, border and margin, each top, right, bottom and left, as twelve f32s.
    pub extern "C" fn get_box(this: &Tree, node: i64, out: BufferMut) -> bool {
        !this.released
            && ops::box_model(shared(), node as u64).is_some_and(|values| put_floats(&values, out))
    }

    /// Writes `node`'s padding and whether it paints a box of its own, as five f32s.
    pub extern "C" fn box_edges(this: &Tree, node: i64, out: BufferMut) -> bool {
        !this.released
            && ops::box_edges(shared(), node as u64).is_some_and(|values| put_floats(&values, out))
    }

    /// Whether any of `node`'s box is on screen.
    pub extern "C" fn in_view(this: &Tree, node: i64) -> bool {
        !this.released && shared().in_view(id(node as u64))
    }

    /// `node`'s own text alignment, 0 left, 1 centre, 2 right; -1 when it has none.
    pub extern "C" fn text_align(this: &Tree, node: i64) -> i32 {
        if this.released {
            return -1;
        }
        ops::text_align(shared(), node as u64)
    }

    /// Writes the line limit (0 for none) and the text-overflow (-1 undeclared, 0 clip, 1 ellipsis) as two i32s.
    pub extern "C" fn text_limits(this: &Tree, node: i64, out: BufferMut) {
        if this.released || out.len() < 8 {
            return;
        }
        let (lines, overflow) = ops::text_limits(shared(), node as u64);
        let at = out.as_mut_ptr() as *mut i32;
        unsafe {
            at.write_unaligned(lines);
            at.add(1).write_unaligned(overflow);
        }
    }

    /// Applies a `Single` property (`prop` is ashui's `PropertyId`) to `node`; takes effect at the next `flush`.
    pub extern "C" fn apply_f32(this: &Tree, node: i64, prop: i32, value: f32) {
        if !this.released {
            crate::layout_router::apply_f32_const(node as u64, prop, value);
        }
    }

    /// Applies an integer or enum property to `node`; takes effect at the next `flush`.
    pub extern "C" fn apply_i32(this: &Tree, node: i64, prop: i32, value: i32) {
        if !this.released {
            crate::layout_router::apply_i32_const(node as u64, prop, value);
        }
    }

    /// Applies a string property (`None` is the empty text) to `node`; takes effect at the next `flush`.
    pub extern "C" fn apply_string(this: &Tree, node: i64, prop: i32, value: Text) {
        if !this.released {
            let text = (!value.is_null()).then(|| value.as_str().to_string());
            crate::layout_router::apply_string_const(node as u64, prop, text);
        }
    }

    /// Applies a style value (a brush, colour, radius...) to `node`; takes effect at the next `flush`.
    pub extern "C" fn apply_value(this: &Tree, node: i64, prop: i32, value: &StyleValue) {
        if !this.released {
            crate::layout_router::apply_value_const(node as u64, prop, value.0.clone());
        }
    }

    /// Puts a property back to what a new node has.
    pub extern "C" fn unset(this: &Tree, node: i64, prop: i32) {
        if !this.released {
            crate::layout_router::unset(node as u64, prop);
        }
    }

    /// Applies everything written since the last flush; true if something changed what is laid out or drawn.
    pub extern "C" fn flush(this: &Tree) -> bool {
        if this.released {
            return false;
        }
        let tree = shared();
        let handles =
            std::mem::take(&mut *RELEASED_HANDLES.lock().unwrap_or_else(|e| e.into_inner()));
        for handle in handles {
            release_handle(tree, handle);
        }
        crate::shared::apply_pending(tree)
    }

    /// Removes the handle's nodes now rather than when it is collected. Calls on it afterwards do nothing.
    pub extern "C" fn dispose(this: &mut Tree) {
        if !this.released {
            release_handle(shared(), this.id);
            this.released = true;
        }
    }
}

/// A style value a property takes: a colour, a brush, a corner radius.
pub struct StyleValue(Value);

impl StyleValue {
    pub extern "C" fn color(hex: i32, alpha: f32) -> Box<StyleValue> {
        Box::new(StyleValue(Value::Color(
            Color::from_hex(hex as u32).with_alpha(alpha),
        )))
    }

    pub extern "C" fn brush_solid(hex: i32, alpha: f32) -> Box<StyleValue> {
        Box::new(StyleValue(Value::Brush(Brush::Solid(
            Color::from_hex(hex as u32).with_alpha(alpha),
        ))))
    }

    pub extern "C" fn corner_radius(tl: f32, tr: f32, br: f32, bl: f32) -> Box<StyleValue> {
        Box::new(StyleValue(Value::Radius(CornerRadius::new(tl, tr, br, bl))))
    }
}

caribou_abi::plugin! {
    name: "blinc_abi";
    class Tree {
        fn new() -> Box<Tree>;
        fn create_node(&Tree) -> i64;
        fn add_child(&Tree, i64, i64);
        fn remove_node(&Tree, i64);
        fn remove_subtree(&Tree, i64);
        fn clear_children(&Tree, i64);
        fn compute_layout(&Tree, i64, f32, f32);
        fn get_bounds(&Tree, i64, BufferMut) -> bool;
        fn create_text_node(&Tree, Text, Text, f32, f32, i32, i32, i32) -> i64;
        fn replace_node(&Tree, i64, i64);
        fn replace_children(&Tree, i64, Buffer);
        fn set_children(&Tree, i64, Buffer);
        fn set_grid_rows(&Tree, i64, Buffer) -> bool;
        fn children(&Tree, i64, BufferMut) -> i32;
        fn ancestors(&Tree, i64, BufferMut) -> i32;
        fn get_box(&Tree, i64, BufferMut) -> bool;
        fn box_edges(&Tree, i64, BufferMut) -> bool;
        fn in_view(&Tree, i64) -> bool;
        fn text_align(&Tree, i64) -> i32;
        fn text_limits(&Tree, i64, BufferMut);
        fn apply_f32(&Tree, i64, i32, f32);
        fn apply_i32(&Tree, i64, i32, i32);
        fn apply_string(&Tree, i64, i32, Text);
        fn apply_value(&Tree, i64, i32, &StyleValue);
        fn unset(&Tree, i64, i32);
        fn flush(&Tree) -> bool;
        fn dispose(&mut Tree);
    }
    class StyleValue {
        fn color(i32, f32) -> Box<StyleValue>;
        fn brush_solid(i32, f32) -> Box<StyleValue>;
        fn corner_radius(f32, f32, f32, f32) -> Box<StyleValue>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Width and height written as constants take effect at the flush, are laid out, and an unset
    /// puts the width back to a new node's.
    #[test]
    fn written_sizes_are_flushed_laid_out_and_unset() {
        const WIDTH: i32 = 10;
        const HEIGHT: i32 = 11;
        let tree = Tree::new();
        let root = Tree::create_node(&tree);
        let child = Tree::create_node(&tree);
        Tree::add_child(&tree, root, child);
        for (node, w, h) in [(root, 200.0, 100.0), (child, 50.0, 20.0)] {
            Tree::apply_f32(&tree, node, WIDTH, w);
            Tree::apply_f32(&tree, node, HEIGHT, h);
        }
        assert!(Tree::flush(&tree));
        Tree::compute_layout(&tree, root, 800.0, 600.0);
        let bounds = |node| {
            let b = shared()
                .layout
                .get_absolute_bounds(id(node as u64))
                .unwrap();
            (b.width, b.height)
        };
        assert_eq!(bounds(root), (200.0, 100.0));
        assert_eq!(bounds(child), (50.0, 20.0));
        Tree::apply_f32(&tree, child, WIDTH, 80.0);
        Tree::flush(&tree);
        Tree::compute_layout(&tree, root, 800.0, 600.0);
        assert_eq!(bounds(child).0, 80.0);
        Tree::unset(&tree, child, WIDTH);
        Tree::flush(&tree);
        Tree::compute_layout(&tree, root, 800.0, 600.0);
        assert_ne!(bounds(child).0, 80.0);
        let mut tree = tree;
        Tree::dispose(&mut tree);
        assert_eq!(Tree::create_node(&tree), 0);
    }
}
