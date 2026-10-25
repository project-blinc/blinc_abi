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
use crate::types::{
    self, Value, blur_value, clip_path_value, clip_polygon_value, glass_value, gradient_value,
    hex_color, image_mapping, image_value, linear_gradient_value, shadow_value,
    text_decoration_value,
};
use blinc_core::{Brush, CornerRadius, Transform};
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
        Box::new(StyleValue(Value::Color(hex_color(hex, alpha))))
    }

    pub extern "C" fn brush_solid(hex: i32, alpha: f32) -> Box<StyleValue> {
        Box::new(StyleValue(Value::Brush(Brush::Solid(hex_color(
            hex, alpha,
        )))))
    }

    /// A frosted-glass brush; `simple` nonzero is the cheaper blur.
    pub extern "C" fn brush_glass(
        blur: f32,
        tint_hex: i32,
        tint_alpha: f32,
        simple: i32,
        noise: f32,
    ) -> Box<StyleValue> {
        Box::new(StyleValue(glass_value(
            blur,
            tint_hex,
            tint_alpha,
            simple != 0,
            noise,
        )))
    }

    /// Sets a glass brush's aberration, 0 to 1, while it is being built.
    pub extern "C" fn glass_aberration(this: &mut StyleValue, strength: f32) {
        types::glass_aberration(&mut this.0, strength);
    }

    /// Sets a glass brush's bevel, 0 to 1; `inset` reverses its curvature.
    pub extern "C" fn glass_bevel(this: &mut StyleValue, strength: f32, inset: bool) {
        types::glass_bevel(&mut this.0, strength, inset);
    }

    /// What is behind the box, blurred by `radius`, tinted when `tint_alpha` is above zero.
    pub extern "C" fn brush_blur(radius: f32, tint_hex: i32, tint_alpha: f32) -> Box<StyleValue> {
        Box::new(StyleValue(blur_value(radius, tint_hex, tint_alpha)))
    }

    /// An image brush; `fit` is 0 cover, 1 contain, 2 fill, 3 tile.
    pub extern "C" fn brush_image(src: Text, fit: i32) -> Box<StyleValue> {
        Box::new(StyleValue(image_value(src.as_str().to_string(), fit)))
    }

    /// Positions an image brush: fractions `x`, `y` and pixel offsets `ox`, `oy`, and the repeat axes (0 to 3).
    pub extern "C" fn brush_image_mapping(
        this: &mut StyleValue,
        x: f32,
        y: f32,
        ox: f32,
        oy: f32,
        repeat: i32,
    ) {
        image_mapping(&mut this.0, x, y, ox, oy, repeat);
    }

    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn brush_linear_gradient(
        sx: f32,
        sy: f32,
        ex: f32,
        ey: f32,
        from_hex: i32,
        from_alpha: f32,
        to_hex: i32,
        to_alpha: f32,
    ) -> Box<StyleValue> {
        Box::new(StyleValue(linear_gradient_value(
            sx, sy, ex, ey, from_hex, from_alpha, to_hex, to_alpha,
        )))
    }

    /// A gradient with no stops: linear from (x1, y1) to (x2, y2), or radial about (x1, y1) of radius x2.
    pub extern "C" fn brush_gradient(
        radial: bool,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        bbox: bool,
    ) -> Box<StyleValue> {
        Box::new(StyleValue(gradient_value(radial, x1, y1, x2, y2, bbox)))
    }

    /// Adds a stop at `offset`, 0 to 1, to a gradient brush.
    pub extern "C" fn gradient_stop(this: &mut StyleValue, offset: f32, hex: i32, alpha: f32) {
        types::gradient_stop(&mut this.0, offset, hex, alpha);
    }

    pub extern "C" fn corner_radius(tl: f32, tr: f32, br: f32, bl: f32) -> Box<StyleValue> {
        Box::new(StyleValue(Value::Radius(CornerRadius::new(tl, tr, br, bl))))
    }

    /// Each corner's superellipse exponent, top-left first; `locked` keeps it from the theme's squircle.
    pub extern "C" fn corner_shape(
        tl: f32,
        tr: f32,
        br: f32,
        bl: f32,
        locked: bool,
    ) -> Box<StyleValue> {
        Box::new(StyleValue(Value::CornerShape([tl, tr, br, bl], locked)))
    }

    pub extern "C" fn transform_identity() -> Box<StyleValue> {
        Box::new(StyleValue(Value::Transform(Transform::identity())))
    }

    pub extern "C" fn transform_translate(x: f32, y: f32) -> Box<StyleValue> {
        Box::new(StyleValue(Value::Transform(Transform::translate(x, y))))
    }

    /// The 2D affine `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`.
    pub extern "C" fn transform_affine(
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        tx: f32,
        ty: f32,
    ) -> Box<StyleValue> {
        Box::new(StyleValue(Value::Transform(Transform::Affine2D(
            blinc_core::Affine2D {
                elements: [a, b, c, d, tx, ty],
            },
        ))))
    }

    /// A shadow of one layer, inside the box when `inset`; `shadow_push` adds more.
    pub extern "C" fn shadow(
        offset_x: f32,
        offset_y: f32,
        blur: f32,
        spread: f32,
        hex: i32,
        alpha: f32,
        inset: bool,
    ) -> Box<StyleValue> {
        Box::new(StyleValue(shadow_value(
            offset_x, offset_y, blur, spread, hex, alpha, inset,
        )))
    }

    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn shadow_push(
        this: &mut StyleValue,
        offset_x: f32,
        offset_y: f32,
        blur: f32,
        spread: f32,
        hex: i32,
        alpha: f32,
        inset: bool,
    ) {
        types::shadow_push(
            &mut this.0,
            offset_x,
            offset_y,
            blur,
            spread,
            hex,
            alpha,
            inset,
        );
    }

    /// `lines` is underline, overline and line-through as bits; `style` 0 solid, 1 double, 2 dotted, 3 dashed.
    /// A negative alpha is no colour of its own, a negative thickness or a NaN offset none.
    pub extern "C" fn text_decoration(
        lines: i32,
        style: i32,
        hex: i32,
        alpha: f32,
        thickness: f32,
        offset: f32,
    ) -> Box<StyleValue> {
        Box::new(StyleValue(text_decoration_value(
            lines, style, hex, alpha, thickness, offset,
        )))
    }

    /// A `clip-path` shape (see `types::clip_path_value`); `values` is f32s, four at most. A negative kind is no clip.
    pub extern "C" fn clip_path(
        kind: i32,
        values: Buffer,
        percent: i32,
        none: i32,
        round: f32,
    ) -> Box<StyleValue> {
        let floats = floats_of(values);
        Box::new(StyleValue(clip_path_value(
            kind, &floats, percent, none, round,
        )))
    }

    /// A polygon clip: `count` points as f32 pairs; a nonzero byte of `percent` makes that value a percentage.
    pub extern "C" fn clip_polygon(
        values: Buffer,
        percent: Buffer,
        count: i32,
        path: bool,
    ) -> Box<StyleValue> {
        let floats = floats_of(values);
        let flags = unsafe { percent.as_slice() }.to_vec();
        Box::new(StyleValue(clip_polygon_value(&floats, &flags, count, path)))
    }

    /// The clip filling by the even-odd rule, as a new value; a value that is no clip gives none.
    pub extern "C" fn clip_even_odd(this: &StyleValue) -> Box<StyleValue> {
        Box::new(StyleValue(match &this.0 {
            Value::ClipPath(path, _) => Value::ClipPath(path.clone(), true),
            _ => Value::None,
        }))
    }
}

/// The f32s packed in `buffer`, four bytes each.
fn floats_of(buffer: Buffer) -> Vec<f32> {
    unsafe { buffer.as_slice() }
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
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
        fn brush_glass(f32, i32, f32, i32, f32) -> Box<StyleValue>;
        fn glass_aberration(&mut StyleValue, f32);
        fn glass_bevel(&mut StyleValue, f32, bool);
        fn brush_blur(f32, i32, f32) -> Box<StyleValue>;
        fn brush_image(Text, i32) -> Box<StyleValue>;
        fn brush_image_mapping(&mut StyleValue, f32, f32, f32, f32, i32);
        fn brush_linear_gradient(f32, f32, f32, f32, i32, f32, i32, f32) -> Box<StyleValue>;
        fn brush_gradient(bool, f32, f32, f32, f32, bool) -> Box<StyleValue>;
        fn gradient_stop(&mut StyleValue, f32, i32, f32);
        fn corner_radius(f32, f32, f32, f32) -> Box<StyleValue>;
        fn corner_shape(f32, f32, f32, f32, bool) -> Box<StyleValue>;
        fn transform_identity() -> Box<StyleValue>;
        fn transform_translate(f32, f32) -> Box<StyleValue>;
        fn transform_affine(f32, f32, f32, f32, f32, f32) -> Box<StyleValue>;
        fn shadow(f32, f32, f32, f32, i32, f32, bool) -> Box<StyleValue>;
        fn shadow_push(&mut StyleValue, f32, f32, f32, f32, i32, f32, bool);
        fn text_decoration(i32, i32, i32, f32, f32, f32) -> Box<StyleValue>;
        fn clip_path(i32, Buffer, i32, i32, f32) -> Box<StyleValue>;
        fn clip_polygon(Buffer, Buffer, i32, bool) -> Box<StyleValue>;
        fn clip_even_odd(&StyleValue) -> Box<StyleValue>;
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
