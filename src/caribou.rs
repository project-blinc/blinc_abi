//! The layout tree as a Caribou plugin, beside the HashLink and Node adapters: the same shared
//! tree and node ownership (see `shared`), reached from any Caribou language.
//!
//! A `Tree` value is one handle: the nodes it creates are its own, and dropping or disposing it
//! removes them. Node ids cross as `i64` (a node's raw id), and results that HashLink writes into
//! a caller's buffer are written into a `BufferMut` the same way.

use crate::shared::{
    NEXT_HANDLE, RELEASED_HANDLES, clear_children as clear, forget, id, release_handle, shared,
    subtree,
};
use caribou_abi::BufferMut;
use std::sync::atomic::Ordering;
use taffy::prelude::{AvailableSpace, Size, Style};

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

    /// Removes the handle's nodes now rather than when it is collected. Calls on it afterwards do nothing.
    pub extern "C" fn dispose(this: &mut Tree) {
        if !this.released {
            release_handle(shared(), this.id);
            this.released = true;
        }
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
        fn dispose(&mut Tree);
    }
}
