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
use blinc_layout::div::GenericFont;
use blinc_layout::tree::{LayoutNodeId, TextMeasureContext};
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
    let context = TextMeasureContext {
        content: unsafe { string_from(content) },
        font_size,
        line_height,
        letter_spacing: 0.0,
        wrap: flags & 1 != 0,
        // No family of its own is the system face: the platform's UI face where it is installed.
        font_name: unsafe { opt_string_from(font_name) }.or_else(|| {
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
        font_weight: font_weight.clamp(1, 1000) as u16,
        italic: flags & 2 != 0,
    };
    crate::text::ensure_face(&context);
    let node = tree.layout.create_text_node(Style::default(), context);
    tree.owners.insert(node, unsafe { owner(h) });
    node.to_raw()
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
    let Some(tree) = (unsafe { tree(h) }) else {
        return;
    };
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
    if children.is_null() || len <= 0 {
        clear_children(tree, id(parent));
        return;
    }
    let ids = (0..len as usize)
        .map(|i| id(unsafe { (children as *const u64).add(i).read_unaligned() }))
        .collect();
    tree.layout.replace_children(id(parent), ids);
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
    if len < 0 || len >= i16::MAX as i32 || (len > 0 && rows.is_null()) {
        return false;
    }
    let owner = unsafe { owner(h) };
    let Some(tree) = (unsafe { tree(h) }) else {
        return false;
    };
    let group = id(group);
    let rows: Vec<_> = (0..len as usize)
        .map(|i| id(unsafe { (rows as *const u64).add(i).read_unaligned() }))
        .collect();
    if tree.owners.get(&group) != Some(&owner)
        || rows.iter().any(|r| tree.owners.get(r) != Some(&owner))
    {
        return false;
    }
    tree.layout.set_grid_rows(group, &rows).is_ok()
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
    if !tree.owners.contains_key(&id(node)) {
        return 0;
    }
    let ids: Vec<u64> = tree
        .layout
        .children(id(node))
        .iter()
        .map(|c| c.to_raw())
        .collect();
    if !out.is_null() {
        let n = ids.len().min(capacity.max(0) as usize);
        unsafe { std::ptr::copy_nonoverlapping(ids.as_ptr(), out as *mut u64, n) };
    }
    ids.len() as i32
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
    if !tree.owners.contains_key(&id(node)) {
        return 0;
    }
    let ids: Vec<u64> = tree
        .layout
        .ancestors(id(node))
        .iter()
        .map(|a| a.to_raw())
        .collect();
    if !out.is_null() {
        let n = ids.len().min(capacity.max(0) as usize);
        unsafe { std::ptr::copy_nonoverlapping(ids.as_ptr(), out as *mut u64, n) };
    }
    ids.len() as i32
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
    let parent = id(parent);
    if !tree.owners.contains_key(&parent) {
        return;
    }
    let ids: Vec<LayoutNodeId> = if children.is_null() || len <= 0 {
        Vec::new()
    } else {
        (0..len as usize)
            .map(|i| id(unsafe { (children as *const u64).add(i).read_unaligned() }))
            .filter(|c| tree.owners.contains_key(c))
            .collect()
    };
    // A child still listed under another parent is taken from it first.
    for &child in &ids {
        if let Some(&old) = tree.layout.ancestors(child).first()
            && old != parent
        {
            let kept = tree
                .layout
                .children(old)
                .into_iter()
                .filter(|&c| c != child)
                .collect();
            tree.layout.replace_children(old, kept);
        }
    }
    tree.layout.replace_children(parent, ids);
}
define_prim!(
    hlp_blinc_tree_set_children,
    hl_blinc_tree_set_children,
    "PXblinc_tree_lBi_v"
);

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
    let mut lines = None;
    let mut overflow = None;
    if let Some(tree) = unsafe { tree(h) }
        && (!tree.line_clamps.is_empty() || !tree.text_overflows.is_empty())
    {
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
    unsafe {
        (out as *mut i32).write_unaligned(lines.unwrap_or(0) as i32);
        (out as *mut i32)
            .add(1)
            .write_unaligned(overflow.map_or(-1, i32::from));
    }
}
define_prim!(
    hlp_blinc_tree_text_limits,
    hl_blinc_tree_text_limits,
    "PXblinc_tree_lB_v"
);

/// One walk of a frame: its records and polygon points, and whether its glyphs overflowed the atlases.
struct Attempt {
    records: Vec<f32>,
    points: Vec<f32>,
    atlas_full: bool,
}

/// A frame whose glyphs overflow the atlases starts them afresh, once. The second walk stands even
/// if it overflows too: its records name the glyphs that did fit, so they must stay in the atlases,
/// and the glyphs that did not are left out.
fn build_with_retry<R>(
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
    let display_scale = p(0);
    let shapes = crate::display_list::Shapes {
        n: p(1),
        threshold: p(2),
        radius_full: p(3),
    };
    let text_color = [p(4), p(5), p(6), p(7)];
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
    if !params.is_null() {
        unsafe { (params as *mut f32).add(8).write_unaligned(drawn as f32) };
    }
    let count = records.len() / RECORD_FLOATS;
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
        if slot < 0 {
            tree.images.remove(&id(node));
            tree.image_mappings.remove(&id(node));
        } else {
            tree.images.insert(id(node), slot);
        }
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
        tree.set_canvas(id(node), (slot >= 0).then_some(slot));
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
    let Some(b) = tree.layout.get_absolute_bounds(id(node)) else {
        return false;
    };
    if out.is_null() {
        return false;
    }
    let out = out as *mut f32;
    for (i, v) in [b.x, b.y, b.width, b.height].into_iter().enumerate() {
        unsafe { out.add(i).write_unaligned(v) };
    }
    true
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
    let Some(l) = tree.layout.get_layout(id(node)) else {
        return false;
    };
    if out.is_null() {
        return false;
    }
    let out = out as *mut f32;
    let sides = |r: taffy::Rect<f32>| [r.top, r.right, r.bottom, r.left];
    for (i, v) in [sides(l.padding), sides(l.border), sides(l.margin)]
        .concat()
        .into_iter()
        .enumerate()
    {
        unsafe { out.add(i).write_unaligned(v) };
    }
    true
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
    let Some(layout) = tree.layout.get_layout(id(node)) else {
        return false;
    };
    if out.is_null() {
        return false;
    }
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
    let out = out as *mut f32;
    for (i, v) in [
        p.top,
        p.right,
        p.bottom,
        p.left,
        if painted { 1.0 } else { 0.0 },
    ]
    .into_iter()
    .enumerate()
    {
        unsafe { out.add(i).write_unaligned(v) };
    }
    true
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
    match tree.props.get(&id(node)).and_then(|p| p.text_align) {
        Some(blinc_layout::div::TextAlign::Left) => 0,
        Some(blinc_layout::div::TextAlign::Center) => 1,
        Some(blinc_layout::div::TextAlign::Right) => 2,
        None => -1,
    }
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
    if data.is_null() {
        tree.notches.remove(&id(node));
        return;
    }
    let f = data as *const f32;
    let mut rows = [[0.0f32; 4]; 3];
    for (i, row) in rows.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = unsafe { f.add(i * 4 + j).read_unaligned() };
        }
    }
    tree.notches.insert(id(node), rows);
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
    let Some(tree) = (unsafe { tree(h) }) else {
        return;
    };
    if clear {
        tree.visuals.remove(&id(node));
    } else {
        tree.visuals.insert(id(node), [dx, dy, w, hh]);
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
        tree.set_pointer_events(id(node), Some(!through));
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
    let Some(tree) = (unsafe { tree(h) }) else {
        return;
    };
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
        let c = |shift: i32| ((thumb >> shift) & 0xff) as f32 / 255.0;
        tree.scrolls.insert(
            id(node),
            Scroll {
                x,
                y,
                thumb: [c(16), c(8), c(0), c(24)],
            },
        );
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
    let Some(layout) = tree.layout.get_layout(id(node)) else {
        return false;
    };
    if out.is_null() {
        return false;
    }
    let b = layout.border;
    let view = (
        layout.size.width - b.left - b.right,
        layout.size.height - b.top - b.bottom,
    );
    let out = out as *mut f32;
    for (i, v) in [
        view.0,
        view.1,
        layout.scrollable_overflow_rect.right,
        layout.scrollable_overflow_rect.bottom,
    ]
    .into_iter()
    .enumerate()
    {
        unsafe { out.add(i).write_unaligned(v) };
    }
    true
}
define_prim!(
    hlp_blinc_tree_scroll_extent,
    hl_blinc_tree_scroll_extent,
    "PXblinc_tree_lB_b"
);
