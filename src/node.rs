//! The layout tree as a Haxe handle, and the per-frame steps that drive it:
//! applying queued property writes, computing layout, reading bounds.
//!
//! Every handle is a view onto one Blinc tree for the whole process. Blinc
//! keeps one queue of property writes and one binding registry, both keyed
//! by node id, and node ids are unique only within a Blinc tree; with one
//! tree they are unique everywhere, so a write or binding can never reach
//! another handle's node. A handle owns the nodes it created, and dropping
//! or disposing it removes them.

use crate::hl::{handle_mut, into_handle, opt_string_from, string_from, take_handle};
use crate::reactive::collect_released;
use hl_abi::{define_prim, vbyte};
use std::ffi::c_void;
use taffy::prelude::{AvailableSpace, Size, Style};

use crate::shared::{
    NEXT_HANDLE, RELEASED_HANDLES, clear_children, forget, id, release_handle, shared, subtree,
};
pub use crate::tree::{Scroll, Tree};

/// What a Haxe `blinc_tree` handle holds: which of the shared tree's nodes
/// are its own.
struct TreeHandle {
    id: u64,
    /// Its nodes are already gone: disposed, so its drop queues nothing.
    released: bool,
}

impl Drop for TreeHandle {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        RELEASED_HANDLES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(self.id);
    }
}

/// The shared tree, through a live handle; `None` for a null or disposed one.
///
/// # Safety
/// `h` must be null or a `blinc_tree` handle.
pub(crate) unsafe fn tree<'a>(h: *mut c_void) -> Option<&'a mut Tree> {
    unsafe { handle_mut::<TreeHandle>(h) }.map(|_| shared())
}

/// The id of the handle `h`, for the nodes it creates.
unsafe fn owner(h: *mut c_void) -> u64 {
    unsafe { handle_mut::<TreeHandle>(h) }.map_or(0, |t| t.id)
}

// ============================================================================
// LIFECYCLE AND NODES
// ============================================================================

/// Blinc measures text with real fonts once its measurer is installed, which
/// must happen before the first tree holds text; without it every width is an
/// estimate. Installed once per process.

#[unsafe(no_mangle)]
pub extern "C" fn hl_blinc_tree_new() -> *mut c_void {
    // The renderer's own registry, so layout measures with the faces text is drawn with.
    crate::text::initialize();
    shared();
    into_handle(TreeHandle {
        id: NEXT_HANDLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        released: false,
    })
}
define_prim!(hlp_blinc_tree_new, hl_blinc_tree_new, "P_Xblinc_tree_");

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_create_node(h: *mut c_void) -> u64 {
    let Some(tree) = (unsafe { tree(h) }) else {
        return 0;
    };
    let node = tree.layout.create_node(Style::default());
    tree.owners.insert(node, unsafe { owner(h) });
    node.to_raw()
}
define_prim!(
    hlp_blinc_tree_create_node,
    hl_blinc_tree_create_node,
    "PXblinc_tree__l"
);

/// `flags` bit 0 is wrap and bit 1 italic; they share an argument because
/// Ash's interpreter passes at most eight arguments to a native taking floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_create_text_node(
    h: *mut c_void,
    content: *const vbyte,
    font_name: *const vbyte,
    font_size: f32,
    line_height: f32,
    font_weight: i32,
    generic_font: i32,
    flags: i32,
) -> u64 {
    let Some(tree) = (unsafe { tree(h) }) else {
        return 0;
    };
    let spec = crate::ops::TextSpec {
        content: unsafe { string_from(content) },
        font_name: unsafe { opt_string_from(font_name) },
        font_size,
        line_height,
        font_weight,
        generic_font,
        flags,
    };
    crate::ops::create_text_node(tree, unsafe { owner(h) }, spec)
}
define_prim!(
    hlp_blinc_tree_create_text_node,
    hl_blinc_tree_create_text_node,
    "PXblinc_tree_BBffiii_l"
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_add_child(h: *mut c_void, parent: u64, child: u64) {
    if let Some(tree) = unsafe { tree(h) } {
        tree.layout.add_child(id(parent), id(child));
    }
}
define_prim!(
    hlp_blinc_tree_add_child,
    hl_blinc_tree_add_child,
    "PXblinc_tree_ll_v"
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_remove_node(h: *mut c_void, node: u64) {
    if let Some(tree) = unsafe { tree(h) } {
        forget(tree, &[id(node)]);
        tree.layout.remove_node(id(node));
        tree.props.remove(&id(node));
    }
}
define_prim!(
    hlp_blinc_tree_remove_node,
    hl_blinc_tree_remove_node,
    "PXblinc_tree_l_v"
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_remove_subtree(h: *mut c_void, node: u64) {
    if let Some(tree) = unsafe { tree(h) } {
        let nodes = subtree(&tree.layout, id(node));
        forget(tree, &nodes);
        tree.layout.remove_subtree(id(node));
        tree.pruned = true;
    }
}
define_prim!(
    hlp_blinc_tree_remove_subtree,
    hl_blinc_tree_remove_subtree,
    "PXblinc_tree_l_v"
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_clear_children(h: *mut c_void, parent: u64) {
    if let Some(tree) = unsafe { tree(h) } {
        clear_children(tree, id(parent));
    }
}
define_prim!(
    hlp_blinc_tree_clear_children,
    hl_blinc_tree_clear_children,
    "PXblinc_tree_l_v"
);

/// Put `new` where `old` is among its parent's children, leaving `old`
/// detached but not deleted. Nothing happens when `old` has no parent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_replace_node(h: *mut c_void, old: u64, new: u64) {
    if let Some(tree) = unsafe { tree(h) } {
        crate::ops::replace_node(tree, old, new);
    }
}
define_prim!(
    hlp_blinc_tree_replace_node,
    hl_blinc_tree_replace_node,
    "PXblinc_tree_ll_v"
);

/// `children` is `len` consecutive 64-bit node ids.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_replace_children(
    h: *mut c_void,
    parent: u64,
    children: *const vbyte,
    len: i32,
) {
    let Some(tree) = (unsafe { tree(h) }) else {
        return;
    };
    let ids = unsafe { ids_from(children, len) };
    crate::ops::replace_children(tree, parent, &ids);
}
define_prim!(
    hlp_blinc_tree_replace_children,
    hl_blinc_tree_replace_children,
    "PXblinc_tree_lBi_v"
);

/// Opt into shared layout/paint for direct rows. `rows` contains `len` u64 ids.
/// An empty list resets the projection; invalid ownership/structure leaves it unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_grid_rows(
    h: *mut c_void,
    group: u64,
    rows: *const vbyte,
    len: i32,
) -> bool {
    if len < 0 || (len > 0 && rows.is_null()) {
        return false;
    }
    let owner = unsafe { owner(h) };
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    let rows = unsafe { ids_from(rows, len) };
    crate::ops::set_grid_rows(tree, owner, group, &rows)
}
define_prim!(
    hlp_blinc_tree_set_grid_rows,
    hl_blinc_tree_set_grid_rows,
    "PXblinc_tree_lBi_b"
);

/// Writes `node`'s children as 64-bit ids into `out`, at most `capacity`.
/// Returns how many there are; 0 for a node that is gone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_children(
    h: *mut c_void,
    node: u64,
    out: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(tree) = (unsafe { tree(h) }) else {
        return 0;
    };
    let Some(ids) = crate::ops::children(tree, node) else {
        return 0;
    };
    unsafe { write_ids(&ids, out, capacity) }
}
define_prim!(
    hlp_blinc_tree_children,
    hl_blinc_tree_children,
    "PXblinc_tree_lBi_i"
);

/// Writes `node`'s ancestors as 64-bit ids into `out`, the parent first, at
/// most `capacity`. Returns how many there are; 0 for a node that is gone or
/// has no parent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_ancestors(
    h: *mut c_void,
    node: u64,
    out: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(tree) = (unsafe { tree(h) }) else {
        return 0;
    };
    let Some(ids) = crate::ops::ancestors(tree, node) else {
        return 0;
    };
    unsafe { write_ids(&ids, out, capacity) }
}
define_prim!(
    hlp_blinc_tree_ancestors,
    hl_blinc_tree_ancestors,
    "PXblinc_tree_lBi_i"
);

/// Makes `children`, `len` consecutive 64-bit ids, `parent`'s children in
/// that order. Children it had before and does not keep are detached, not
/// deleted, as are `children` from any other parent. A `parent` that is gone
/// is left alone, and ids of nodes that are gone are skipped: a parent's
/// removal can come before the update of what was placed in it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_children(
    h: *mut c_void,
    parent: u64,
    children: *const vbyte,
    len: i32,
) {
    let Some(tree) = (unsafe { tree(h) }) else {
        return;
    };
    let ids = unsafe { ids_from(children, len) };
    crate::ops::set_children(tree, parent, &ids);
}
define_prim!(
    hlp_blinc_tree_set_children,
    hl_blinc_tree_set_children,
    "PXblinc_tree_lBi_v"
);

/// `len` consecutive 64-bit node ids at `ids`; none for a null pointer or a length of zero or less.
unsafe fn ids_from(ids: *const vbyte, len: i32) -> Vec<u64> {
    if ids.is_null() || len <= 0 {
        return Vec::new();
    }
    (0..len as usize)
        .map(|i| unsafe { (ids as *const u64).add(i).read_unaligned() })
        .collect()
}

/// Writes at most `capacity` of `ids` to `out`; how many there are.
unsafe fn write_ids(ids: &[u64], out: *mut vbyte, capacity: i32) -> i32 {
    if !out.is_null() {
        let n = ids.len().min(capacity.max(0) as usize);
        unsafe { std::ptr::copy_nonoverlapping(ids.as_ptr(), out as *mut u64, n) };
    }
    ids.len() as i32
}

/// Writes `values` to `out`; false for a null `out`.
unsafe fn write_floats(values: &[f32], out: *mut vbyte) -> bool {
    if out.is_null() {
        return false;
    }
    for (i, v) in values.iter().enumerate() {
        unsafe { (out as *mut f32).add(i).write_unaligned(*v) };
    }
    true
}

fn drain_released_handles() -> Vec<u64> {
    std::mem::take(&mut *RELEASED_HANDLES.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Remove the handle's nodes now rather than when the handle is collected,
/// with their bindings. Calls on the handle afterwards do nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_dispose(h: *mut c_void) {
    if let Some(mut handle) = unsafe { take_handle::<TreeHandle>(h) } {
        release_handle(shared(), handle.id);
        handle.released = true;
    }
}
define_prim!(
    hlp_blinc_tree_dispose,
    hl_blinc_tree_dispose,
    "PXblinc_tree__v"
);

// ============================================================================
// FRAME
// ============================================================================

/// Apply every property write queued since the last flush, and report whether
/// any changed what is drawn: a relayout, or a visual property such as an
/// opacity or a colour, which needs a frame but no layout. Signals and computeds whose handles were
/// collected since the last flush are removed from Blinc's graph first. The
/// writes are the whole process's, applied to the one tree every handle shares.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_flush(h: *mut c_void) -> bool {
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    // Unbinding first frees the computeds bound only to those nodes.
    for handle in drain_released_handles() {
        release_handle(tree, handle);
    }
    collect_released();
    crate::shared::apply_pending(tree)
}
define_prim!(hlp_blinc_tree_flush, hl_blinc_tree_flush, "PXblinc_tree__b");

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_compute_layout(
    h: *mut c_void,
    root: u64,
    width: f32,
    height: f32,
) {
    if let Some(tree) = unsafe { tree(h) } {
        let space = Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        };
        tree.sync_line_clamps();
        tree.layout.compute_layout(id(root), space);
    }
}
define_prim!(
    hlp_blinc_tree_compute_layout,
    hl_blinc_tree_compute_layout,
    "PXblinc_tree_lff_v"
);

/// Text limits of this node or its nearest declaring ancestor: line count (0 for none)
/// and text-overflow (-1 undeclared, 0 clip, 1 ellipsis). Inline-flow hosts read these.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_text_limits(h: *mut c_void, node: u64, out: *mut vbyte) {
    if out.is_null() {
        return;
    }
    let (lines, overflow) = match unsafe { tree(h) } {
        Some(tree) => crate::ops::text_limits(tree, node),
        None => (0, -1),
    };
    unsafe {
        (out as *mut i32).write_unaligned(lines);
        (out as *mut i32).add(1).write_unaligned(overflow);
    }
}
define_prim!(
    hlp_blinc_tree_text_limits,
    hl_blinc_tree_text_limits,
    "PXblinc_tree_lB_v"
);

/// Pack the primitives to draw under `root` into `out` (see `display_list`), at
/// most `capacity` records. `params` is eight f32s: the device pixels per
/// layout unit text is rasterized for; the theme's corner `n` (0 for no
/// smoothing), smoothing threshold and full radius; and the colour, straight
/// RGBA, of text that neither it nor an ancestor sets; and room for a ninth,
/// where the number of records to draw is written. Returns how many records'
/// room the list takes, polygon points after the records included, which may
/// be more than were written: the caller grows its buffer and asks again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_display_list(
    h: *mut c_void,
    root: u64,
    params: *const vbyte,
    out: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(tree) = (unsafe { tree(h) }) else {
        return 0;
    };
    let p = |i: usize| -> f32 {
        if params.is_null() {
            return 0.0;
        }
        unsafe { (params as *const f32).add(i).read_unaligned() }
    };
    let (records, drawn) =
        crate::ops::display_list(tree, root, [p(0), p(1), p(2), p(3), p(4), p(5), p(6), p(7)]);
    if !params.is_null() {
        unsafe { (params as *mut f32).add(8).write_unaligned(drawn as f32) };
    }
    let count = records.len() / crate::display_list::RECORD_FLOATS;
    let written = count.min(capacity.max(0) as usize) * crate::display_list::RECORD_FLOATS;
    if !out.is_null() && written > 0 {
        unsafe { std::ptr::copy_nonoverlapping(records.as_ptr(), out as *mut f32, written) };
    }
    count as i32
}
define_prim!(
    hlp_blinc_tree_display_list,
    hl_blinc_tree_display_list,
    "PXblinc_tree_lBBi_i"
);

/// Makes `node` draw the image the caller knows as `slot` in its content
/// box, as Blinc's image and SVG elements do; a negative `slot` stops it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_image(h: *mut c_void, node: u64, slot: i32) {
    if let Some(tree) = unsafe { tree(h) } {
        crate::ops::set_image(tree, node, slot);
    }
}
define_prim!(
    hlp_blinc_tree_set_image,
    hl_blinc_tree_set_image,
    "PXblinc_tree_li_v"
);

/// Optional bitmap object fitting and position. Uses numeric enums and f32 geometry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_image_mapping(
    h: *mut c_void,
    node: u64,
    fit: i32,
    x: f32,
    y: f32,
    offset_x: f32,
    offset_y: f32,
) {
    if let Some(tree) = unsafe { tree(h) } {
        tree.set_image_mapping(id(node), fit, [x, y, offset_x, offset_y]);
    }
}
define_prim!(
    hlp_blinc_tree_set_image_mapping,
    hl_blinc_tree_set_image_mapping,
    "PXblinc_tree_liffff_v"
);

/// Makes `node` paint itself in its content box, as the canvas the caller
/// knows as `slot`; a negative `slot` stops it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_canvas(h: *mut c_void, node: u64, slot: i32) {
    if let Some(tree) = unsafe { tree(h) } {
        crate::ops::set_canvas(tree, node, slot);
    }
}
define_prim!(
    hlp_blinc_tree_set_canvas,
    hl_blinc_tree_set_canvas,
    "PXblinc_tree_li_v"
);

/// Write the node's absolute `x, y, width, height` as four f32s into `out`.
/// False, with `out` untouched, for a node the tree has not laid out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_get_bounds(
    h: *mut c_void,
    node: u64,
    out: *mut vbyte,
) -> bool {
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    let Some(values) = crate::ops::bounds(tree, node) else {
        return false;
    };
    unsafe { write_floats(&values, out) }
}
define_prim!(
    hlp_blinc_tree_get_bounds,
    hl_blinc_tree_get_bounds,
    "PXblinc_tree_lB_b"
);

/// `node`'s box model as layout resolved it, in layout units: its padding,
/// border and margin, each top, right, bottom and left, as twelve `f32`s in
/// `out`. False before it has been laid out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_get_box(h: *mut c_void, node: u64, out: *mut vbyte) -> bool {
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    let Some(values) = crate::ops::box_model(tree, node) else {
        return false;
    };
    unsafe { write_floats(&values, out) }
}
define_prim!(
    hlp_blinc_tree_get_box,
    hl_blinc_tree_get_box,
    "PXblinc_tree_lB_b"
);

/// Whether any of `node`'s box is on screen: `Tree::in_view`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_in_view(h: *mut c_void, node: u64) -> bool {
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    tree.in_view(id(node))
}
define_prim!(
    hlp_blinc_tree_in_view,
    hl_blinc_tree_in_view,
    "PXblinc_tree_l_b"
);

/// `node`'s padding as laid out, top, right, bottom, left, then 1 if it
/// paints a box of its own (a background, a border or a shadow) and 0 if not,
/// as five f32s in `out`. False when it has no layout.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_box_edges(
    h: *mut c_void,
    node: u64,
    out: *mut vbyte,
) -> bool {
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    let Some(values) = crate::ops::box_edges(tree, node) else {
        return false;
    };
    unsafe { write_floats(&values, out) }
}
define_prim!(
    hlp_blinc_tree_box_edges,
    hl_blinc_tree_box_edges,
    "PXblinc_tree_lB_b"
);

/// `node`'s own text alignment, 0 left, 1 centre, 2 right, as `ashui.types.Style.TextAlign`; -1 when it has none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_text_align(h: *mut c_void, node: u64) -> i32 {
    let Some(tree) = (unsafe { tree(h) }) else {
        return -1;
    };
    crate::ops::text_align(tree, node)
}
define_prim!(
    hlp_blinc_tree_text_align,
    hl_blinc_tree_text_align,
    "PXblinc_tree_l_i"
);

/// Draws `node` as a notch, from twelve f32s in `data`: its corner radii,
/// top-left, top-right, bottom-right, bottom-left, each negative for a
/// concave corner, then its top and bottom edges, each (kind, width, height,
/// corner radius) with kind 0 none, 1 scoop, 2 bulge, 3 cut, 4 peak. A null
/// `data` draws it as a box again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_notch(h: *mut c_void, node: u64, data: *const vbyte) {
    let Some(tree) = (unsafe { tree(h) }) else {
        return;
    };
    let rows = if data.is_null() {
        None
    } else {
        let mut d = [0.0f32; 12];
        for (i, v) in d.iter_mut().enumerate() {
            *v = unsafe { (data as *const f32).add(i).read_unaligned() };
        }
        Some(d)
    };
    crate::ops::set_notch(tree, node, rows);
}
define_prim!(
    hlp_blinc_tree_set_notch,
    hl_blinc_tree_set_notch,
    "PXblinc_tree_lB_v"
);

/// Draws `node` moved by `(dx, dy)` from its layout, and at size `(w, h)`
/// with its children clipped to it when `w` is not negative, as a layout
/// animation does; its children move with it, and the hit test follows.
/// `clear` draws it at its layout again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_visual(
    h: *mut c_void,
    node: u64,
    dx: f32,
    dy: f32,
    w: f32,
    hh: f32,
    clear: bool,
) {
    if let Some(tree) = unsafe { tree(h) } {
        crate::ops::set_visual(tree, node, dx, dy, w, hh, clear);
    }
}
define_prim!(
    hlp_blinc_tree_set_visual,
    hl_blinc_tree_set_visual,
    "PXblinc_tree_lffffb_v"
);

/// Makes the hit test pass through `node` and everything inside it, or not.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_pass_through(h: *mut c_void, node: u64, through: bool) {
    if let Some(tree) = unsafe { tree(h) } {
        crate::ops::set_pass_through(tree, node, through);
    }
}
define_prim!(
    hlp_blinc_tree_set_pass_through,
    hl_blinc_tree_set_pass_through,
    "PXblinc_tree_lb_v"
);

/// Styles an overlay scrollbar without changing its offsets or interaction.
/// Width zero hides the bar. Track is straight-alpha 0xAARRGGBB; zero is transparent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_scrollbar_style(
    h: *mut c_void,
    node: u64,
    width: f32,
    track: i32,
) {
    if let Some(tree) = unsafe { tree(h) } {
        crate::ops::set_scrollbar_style(tree, node, width, track);
    }
}
define_prim!(
    hlp_blinc_tree_set_scrollbar_style,
    hl_blinc_tree_set_scrollbar_style,
    "PXblinc_tree_lfi_v"
);

/// Scrolls container `node`'s content by `(x, y)` and colours its thumb
/// `thumb`, `0xAARRGGBB` (0 hides it), as Blinc's scroll containers do.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_set_scroll(
    h: *mut c_void,
    node: u64,
    x: f32,
    y: f32,
    thumb: i32,
) {
    if let Some(tree) = unsafe { tree(h) } {
        crate::ops::set_scroll(tree, node, x, y, thumb);
    }
}
define_prim!(
    hlp_blinc_tree_set_scroll,
    hl_blinc_tree_set_scroll,
    "PXblinc_tree_lffi_v"
);

/// Writes container `node`'s viewport width and height (inside its border)
/// and its content's width and height as four f32s into `out`; false before
/// it is laid out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_scroll_extent(
    h: *mut c_void,
    node: u64,
    out: *mut vbyte,
) -> bool {
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    let Some(values) = crate::ops::scroll_extent(tree, node) else {
        return false;
    };
    unsafe { write_floats(&values, out) }
}
define_prim!(
    hlp_blinc_tree_scroll_extent,
    hl_blinc_tree_scroll_extent,
    "PXblinc_tree_lB_b"
);
