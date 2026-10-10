//! Which nodes are under a point, found the way the paint walk draws them:
//! through each node's transform, inside the clips its ancestors push, last
//! child on top. Pointer events in ashui are dispatched from this.

use crate::display_list::{Affine, IDENTITY, apply, compose};
use crate::tree::Tree;
use blinc_core::Transform;
use blinc_layout::tree::LayoutNodeId;
#[cfg(feature = "hashlink")]
use hl_abi::{define_prim, vbyte};
#[cfg(feature = "hashlink")]
use std::ffi::c_void;
use taffy::Overflow;

/// `m` undone, or `None` when `m` flattens the plane.
fn invert(m: Affine) -> Option<Affine> {
    let [a, b, c, d, e, f] = m;
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return None;
    }
    let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
    Some([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)])
}

/// A clip on the way down: a rect in layout coordinates, and what takes a
/// point on screen into them.
struct Clip {
    rect: [f32; 4],
    to_layout: Affine,
}

/// A hit node and the point in its own coordinates, from its top-left.
pub struct Hit {
    pub node: LayoutNodeId,
    pub x: f32,
    pub y: f32,
}

/// A conservative screen-space rectangle in which the hit path stays unchanged.
/// Empty bounds mean a curved/rotated boundary needs the ordinary hit test.
#[derive(Clone, Copy, Debug)]
pub struct HitRegion {
    pub bounds: [f32; 4],
    valid: bool,
}

impl HitRegion {
    fn new(enabled: bool) -> Self {
        Self {
            bounds: [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
            valid: enabled,
        }
    }

    fn constrain(&mut self, m: Affine, rect: [f32; 4], px: f32, py: f32, shaped: bool) {
        if !self.valid {
            return;
        }
        let [x, y, w, h] = rect;
        let corners = [
            apply(m, x, y),
            apply(m, x + w, y),
            apply(m, x, y + h),
            apply(m, x + w, y + h),
        ];
        let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, -f32::MAX, -f32::MAX);
        for (x, y) in corners {
            if !x.is_finite() || !y.is_finite() {
                self.valid = false;
                return;
            }
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        // Keep a numerical margin: the ordinary walk applies the inverse
        // transform, whose rounding can put an edge a few ulps from this AABB.
        let magnitude = 1.0
            + left.abs()
            + top.abs()
            + right.abs()
            + bottom.abs()
            + m[4].abs()
            + m[5].abs()
            + (x.abs() + y.abs() + w.abs() + h.abs())
                * (m[0].abs() + m[1].abs() + m[2].abs() + m[3].abs());
        let margin = 16.0 * f32::EPSILON * magnitude;
        let outside =
            px < left - margin || px > right + margin || py < top - margin || py > bottom + margin;
        if !outside && (shaped || m[1] != 0.0 || m[2] != 0.0) {
            self.valid = false;
            return;
        }
        // Boundary bands fall back to exact hits, including reflected edges.
        for (point, min, max, low, high) in [(px, left, right, 0, 2), (py, top, bottom, 1, 3)] {
            if point < min - margin {
                self.bounds[high] = self.bounds[high].min((min - margin).next_down());
            } else if point > max + margin {
                self.bounds[low] = self.bounds[low].max((max + margin).next_up());
            } else {
                self.bounds[low] = self.bounds[low].max((min + margin).next_up());
                self.bounds[high] = self.bounds[high].min((max - margin).next_down());
            }
        }
    }

    fn finish(mut self, x: f32, y: f32) -> Self {
        let [left, top, right, bottom] = self.bounds;
        if !self.valid
            || !x.is_finite()
            || !y.is_finite()
            || x < left
            || y < top
            || x >= right
            || y >= bottom
        {
            self.bounds = [0.0; 4];
            self.valid = false;
        }
        self
    }
}

/// Appends to `out` the topmost node of `node`'s subtree under `(px, py)` on
/// screen, then each of its ancestors up to `node`; nothing when no node is.
#[allow(clippy::too_many_arguments)]
fn hit(
    tree: &Tree,
    node: LayoutNodeId,
    origin: (f32, f32),
    m: Affine,
    clips: &mut Vec<Clip>,
    px: f32,
    py: f32,
    out: &mut Vec<Hit>,
    region: &mut HitRegion,
) -> bool {
    if tree.pass_through.contains(&node) {
        return false;
    }
    let Some(layout) = tree.layout.get_layout(node) else {
        return false;
    };
    let mut x = origin.0 + layout.location.x;
    let mut y = origin.1 + layout.location.y;
    let (mut w, mut h) = (layout.size.width, layout.size.height);
    // Where a layout animation draws it.
    let mut sized = false;
    if let Some(v) = tree.visuals.get(&node) {
        x += v[0];
        y += v[1];
        if v[2] >= 0.0 {
            (w, h) = (v[2], v[3].max(0.0));
            sized = true;
        }
    }
    let mut m = m;
    if let Some(props) = tree.props.get(&node) {
        if !props.visible {
            return false;
        }
        if let Some(Transform::Affine2D(t)) = &props.transform {
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let about = compose(
                [1.0, 0.0, 0.0, 1.0, cx, cy],
                compose(t.elements, [1.0, 0.0, 0.0, 1.0, -cx, -cy]),
            );
            m = compose(m, about);
        }
    }
    let Some(to_layout) = invert(m) else {
        return false;
    };
    let (lx, ly) = apply(to_layout, px, py);
    let (lx, ly) = (lx - x, ly - y);
    let notch = tree
        .notches
        .get(&node)
        .filter(|n| crate::notch::is_notch(n));
    // A notch's outline is curved and cut: no region over it is cached.
    region.constrain(m, [x, y, w, h], px, py, notch.is_some());
    // Use the resolved shape's bounds, not the element's: an oversized
    // circle or an inset with negative edges can clip overflowing children.
    if let Some(path) = tree.props.get(&node).and_then(|p| p.clip_path.as_ref()) {
        let inside = if region.valid {
            let (inside, bounds) = crate::display_list::shape_hit_test(path, w, h, lx, ly, true);
            if let Some([sx, sy, sw, sh]) = bounds {
                region.constrain(m, [x + sx, y + sy, sw, sh], px, py, true);
            } else {
                region.valid = false;
            }
            inside
        } else {
            crate::display_list::shape_contains(path, w, h, lx, ly)
        };
        if !inside {
            return false;
        }
    }

    let overflow = tree.layout.get_style(node).map(|s| s.overflow);
    let clipped =
        sized || overflow.is_some_and(|o| o.x != Overflow::Visible || o.y != Overflow::Visible);
    if clipped {
        let bw = tree.props.get(&node).map_or(0.0, |p| p.border_width);
        region.constrain(
            m,
            [
                x + bw,
                y + bw,
                (w - 2.0 * bw).max(0.0),
                (h - 2.0 * bw).max(0.0),
            ],
            px,
            py,
            false,
        );
        clips.push(Clip {
            rect: [
                x + bw,
                y + bw,
                (w - 2.0 * bw).max(0.0),
                (h - 2.0 * bw).max(0.0),
            ],
            to_layout,
        });
    }
    let children = tree.layout.children(node);
    let (sx, sy) = tree.scrolls.get(&node).map_or((0.0, 0.0), |s| (s.x, s.y));
    let mut found = false;
    for &child in children.iter().rev() {
        if hit(tree, child, (x - sx, y - sy), m, clips, px, py, out, region) {
            found = true;
            break;
        }
    }
    if clipped {
        clips.pop();
    }
    if !found {
        let inside_clips = clips.iter().all(|c| {
            let (cx, cy) = apply(c.to_layout, px, py);
            let [rx, ry, rw, rh] = c.rect;
            cx >= rx && cy >= ry && cx < rx + rw && cy < ry + rh
        });
        found = inside_clips
            && lx >= 0.0
            && ly >= 0.0
            && lx < w
            && ly < h
            && notch.is_none_or(|n| crate::notch::distance((lx, ly), (w, h), n) < 0.0);
    }
    if found {
        out.push(Hit { node, x: lx, y: ly });
    }
    found
}

/// Writes the nodes under `(x, y)` into `out`, the topmost first and then
/// each ancestor up to `root`, as records of a u64 id and the point in that
/// node's coordinates as two f32s, at most `capacity` of them. Returns how
/// many there are.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_hit_test(
    h: *mut c_void,
    root: u64,
    x: f32,
    y: f32,
    out: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(tree) = (unsafe { crate::node::tree(h) }) else {
        return 0;
    };
    let mut hits = Vec::new();
    hit(
        tree,
        LayoutNodeId::from_raw(root),
        (0.0, 0.0),
        IDENTITY,
        &mut Vec::new(),
        x,
        y,
        &mut hits,
        &mut HitRegion::new(false),
    );
    if !out.is_null() {
        let out = out as *mut u8;
        for (i, hit) in hits.iter().take(capacity.max(0) as usize).enumerate() {
            unsafe {
                let at = out.add(i * 16);
                (at as *mut u64).write_unaligned(hit.node.to_raw());
                (at.add(8) as *mut f32).write_unaligned(hit.x);
                (at.add(12) as *mut f32).write_unaligned(hit.y);
            }
        }
    }
    hits.len() as i32
}
#[cfg(feature = "hashlink")]
define_prim!(
    hlp_blinc_tree_hit_test,
    hl_blinc_tree_hit_test,
    "PXblinc_tree_lffBi_i"
);

fn order(tree: &Tree, node: LayoutNodeId, out: &mut Vec<u64>) {
    if tree.props.get(&node).is_some_and(|p| !p.visible) {
        return;
    }
    out.push(node.to_raw());
    for child in tree.layout.children(node) {
        order(tree, child, out);
    }
}

/// Writes the visible nodes under `root`, `root` first, in document order (a
/// node, then its children's subtrees in turn) into `out` as u64 ids, at
/// most `capacity`. Returns how many there are.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_order(
    h: *mut c_void,
    root: u64,
    out: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(tree) = (unsafe { crate::node::tree(h) }) else {
        return 0;
    };
    let mut ids = Vec::new();
    order(tree, LayoutNodeId::from_raw(root), &mut ids);
    if !out.is_null() {
        let n = ids.len().min(capacity.max(0) as usize);
        unsafe { std::ptr::copy_nonoverlapping(ids.as_ptr(), out as *mut u64, n) };
    }
    ids.len() as i32
}
#[cfg(feature = "hashlink")]
define_prim!(
    hlp_blinc_tree_order,
    hl_blinc_tree_order,
    "PXblinc_tree_lBi_i"
);

fn path(tree: &Tree, from: LayoutNodeId, to: LayoutNodeId, out: &mut Vec<u64>) -> bool {
    if from == to {
        out.push(from.to_raw());
        return true;
    }
    for child in tree.layout.children(from) {
        if path(tree, child, to, out) {
            out.push(from.to_raw());
            return true;
        }
    }
    false
}

/// Writes `node` and its ancestors up to `root`, `node` first, into `out` as
/// u64 ids, at most `capacity`. Returns how many there are: 0 when `node`
/// is not under `root`.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_path(
    h: *mut c_void,
    root: u64,
    node: u64,
    out: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(tree) = (unsafe { crate::node::tree(h) }) else {
        return 0;
    };
    let mut ids = Vec::new();
    path(
        tree,
        LayoutNodeId::from_raw(root),
        LayoutNodeId::from_raw(node),
        &mut ids,
    );
    if !out.is_null() {
        let n = ids.len().min(capacity.max(0) as usize);
        unsafe { std::ptr::copy_nonoverlapping(ids.as_ptr(), out as *mut u64, n) };
    }
    ids.len() as i32
}
#[cfg(feature = "hashlink")]
define_prim!(
    hlp_blinc_tree_path,
    hl_blinc_tree_path,
    "PXblinc_tree_llBi_i"
);

pub fn test(tree: &Tree, root: LayoutNodeId, x: f32, y: f32) -> Vec<Hit> {
    let mut out = Vec::new();
    hit(
        tree,
        root,
        (0.0, 0.0),
        IDENTITY,
        &mut Vec::new(),
        x,
        y,
        &mut out,
        &mut HitRegion::new(false),
    );
    out
}
pub fn paint_order(tree: &Tree, root: LayoutNodeId) -> Vec<u64> {
    let mut out = Vec::new();
    order(tree, root, &mut out);
    out
}
pub fn node_path(tree: &Tree, from: LayoutNodeId, to: LayoutNodeId) -> Vec<u64> {
    let mut out = Vec::new();
    path(tree, from, to, &mut out);
    out
}

/// Hit-test once and return the region safe to reuse until geometry changes.
/// Callers still transform coordinates freshly for actual pointer events.
pub fn test_region(tree: &Tree, root: LayoutNodeId, x: f32, y: f32) -> (Vec<Hit>, HitRegion) {
    let mut out = Vec::new();
    let mut region = HitRegion::new(true);
    hit(
        tree,
        root,
        (0.0, 0.0),
        IDENTITY,
        &mut Vec::new(),
        x,
        y,
        &mut out,
        &mut region,
    );
    (out, region.finish(x, y))
}

/// As hit_test, also writes conservative (left, top, right, bottom) bounds.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_tree_hit_test_region(
    h: *mut c_void,
    root: u64,
    x: f32,
    y: f32,
    out: *mut vbyte,
    capacity: i32,
    bounds: *mut vbyte,
) -> i32 {
    if !bounds.is_null() {
        unsafe { std::ptr::write_bytes(bounds, 0, 16) };
    }
    let Some(tree) = (unsafe { crate::node::tree(h) }) else {
        return 0;
    };
    let (hits, region) = test_region(tree, LayoutNodeId::from_raw(root), x, y);
    if !out.is_null() {
        for (i, hit) in hits.iter().take(capacity.max(0) as usize).enumerate() {
            unsafe {
                let at = (out as *mut u8).add(i * 16);
                (at as *mut u64).write_unaligned(hit.node.to_raw());
                (at.add(8) as *mut f32).write_unaligned(hit.x);
                (at.add(12) as *mut f32).write_unaligned(hit.y);
            }
        }
    }
    if !bounds.is_null() {
        for (i, value) in region.bounds.into_iter().enumerate() {
            unsafe { (bounds as *mut f32).add(i).write_unaligned(value) };
        }
    }
    hits.len() as i32
}
#[cfg(feature = "hashlink")]
define_prim!(
    hlp_blinc_tree_hit_test_region,
    hl_blinc_tree_hit_test_region,
    "PXblinc_tree_lffBiB_i"
);

#[cfg(test)]
mod tests {
    use super::*;
    use blinc_core::{Affine2D, ClipLength, ClipPath};
    use blinc_layout::element::RenderProps;
    use taffy::Point;
    use taffy::prelude::*;

    fn box_at(tree: &mut Tree, x: f32, y: f32, w: f32, h: f32) -> LayoutNodeId {
        tree.create_node(Style {
            position: Position::Absolute,
            inset: Rect {
                left: LengthPercentageAuto::length(x),
                top: LengthPercentageAuto::length(y),
                right: LengthPercentageAuto::AUTO,
                bottom: LengthPercentageAuto::AUTO,
            },
            size: Size {
                width: Dimension::length(w),
                height: Dimension::length(h),
            },
            ..Default::default()
        })
    }
    fn layout(tree: &mut Tree, root: LayoutNodeId) {
        tree.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(200.0),
                height: AvailableSpace::Definite(200.0),
            },
        );
    }
    // Compare every point in each cached cell against the ordinary paint-order
    // walk, including points just inside its f32 boundaries.
    fn verify(tree: &Tree, root: LayoutNodeId) -> usize {
        let mut cached = 0;
        for x in (1..240).step_by(7) {
            for y in (1..220).step_by(11) {
                let (hits, region) = test_region(tree, root, x as f32 + 0.25, y as f32 + 0.5);
                let expected: Vec<_> = hits.iter().map(|h| h.node).collect();
                let [l, t, r, b] = region.bounds;
                if l >= r || t >= b {
                    continue;
                }
                cached += 1;
                let (l, t, r, b) = (l.max(-20.0), t.max(-20.0), r.min(260.0), b.min(260.0));
                for px in [l, l.next_up(), (l + r) / 2.0, r.next_down()] {
                    for py in [t, t.next_up(), (t + b) / 2.0, b.next_down()] {
                        let actual: Vec<_> =
                            test(tree, root, px, py).iter().map(|h| h.node).collect();
                        assert_eq!(
                            expected, actual,
                            "cached {:?} at ({x},{y}) disagrees at ({px},{py})",
                            region.bounds
                        );
                    }
                }
            }
        }
        cached
    }

    #[test]
    fn a_notch_is_hit_by_its_outline() {
        let mut tree = Tree::new();
        let root = box_at(&mut tree, 0.0, 0.0, 200.0, 200.0);
        let dropdown = box_at(&mut tree, 0.0, 0.0, 200.0, 100.0);
        tree.add_child(root, dropdown);
        layout(&mut tree, root);
        // Concave top corners of 20: the body is inset by them, with a flare at each top corner.
        tree.notches
            .insert(dropdown, [[-20.0, -20.0, 10.0, 10.0], [0.0; 4], [0.0; 4]]);
        let top = |x: f32, y: f32| test(&tree, root, x, y).first().map(|h| h.node);
        assert_eq!(top(100.0, 50.0), Some(dropdown));
        assert_eq!(top(10.0, 21.0), Some(dropdown));
        // Above the body and in the flare's cut-out, the press goes to what is beneath.
        assert_eq!(top(100.0, 10.0), Some(root));
        assert_eq!(top(2.0, 30.0), Some(root));
        // No quiet region is cached over the notch.
        assert!(verify(&tree, root) > 0);
    }

    #[test]
    fn quiet_regions_follow_overlap_clips_scroll_and_visual_offsets() {
        let mut tree = Tree::new();
        let root = box_at(&mut tree, 0.0, 0.0, 200.0, 200.0);
        let a = box_at(&mut tree, 20.0, 30.0, 80.0, 60.0);
        let b = box_at(&mut tree, 60.0, 40.0, 80.0, 100.0);
        let overflow = box_at(&mut tree, -15.0, 70.0, 120.0, 50.0);
        tree.add_child(root, a);
        tree.add_child(root, b);
        tree.add_child(b, overflow);
        layout(&mut tree, root);
        assert!(verify(&tree, root) > 500);
        let mut style = tree.get_style(b).unwrap().clone();
        style.overflow = Point {
            x: Overflow::Hidden,
            y: Overflow::Hidden,
        };
        tree.set_style(b, style);
        tree.props.insert(
            b,
            RenderProps {
                border_width: 3.0,
                ..Default::default()
            },
        );
        tree.scrolls.insert(
            b,
            crate::tree::Scroll {
                x: 7.25,
                y: 18.5,
                ..Default::default()
            },
        );
        tree.visuals.insert(a, [11.25, -8.5, 42.0, 22.0]);
        layout(&mut tree, root);
        assert!(verify(&tree, root) > 500);
        tree.pass_through.insert(b);
        assert!(verify(&tree, root) > 500);
    }

    #[test]
    fn reflected_scaled_regions_preserve_hit_paths() {
        let mut tree = Tree::new();
        let root = box_at(&mut tree, 0.0, 0.0, 200.0, 200.0);
        let child = box_at(&mut tree, 20.25, 30.5, 45.25, 65.5);
        tree.add_child(root, child);
        tree.props.insert(
            child,
            RenderProps {
                transform: Some(Transform::Affine2D(Affine2D {
                    elements: [-1.75, 0.0, 0.0, 0.65, 8.5, 11.25],
                })),
                ..Default::default()
            },
        );
        layout(&mut tree, root);
        assert!(verify(&tree, root) > 500);
    }

    #[test]
    fn shaped_and_rotated_bounds_fall_back_inside_but_cache_outside() {
        let mut tree = Tree::new();
        let root = box_at(&mut tree, 0.0, 0.0, 200.0, 200.0);
        let child = box_at(&mut tree, 40.0, 40.0, 80.0, 80.0);
        tree.add_child(root, child);
        tree.props.insert(
            child,
            RenderProps {
                clip_path: Some(ClipPath::Circle {
                    radius: None,
                    center: (ClipLength::Percent(50.0), ClipLength::Percent(50.0)),
                }),
                ..Default::default()
            },
        );
        layout(&mut tree, root);
        assert_eq!(test_region(&tree, root, 80.0, 80.0).1.bounds, [0.0; 4]);
        assert!(verify(&tree, root) > 400);
        tree.props.get_mut(&child).unwrap().transform = Some(Transform::Affine2D(Affine2D {
            elements: [0.707, 0.707, -0.707, 0.707, 0.0, 0.0],
        }));
        assert_eq!(test_region(&tree, root, 80.0, 80.0).1.bounds, [0.0; 4]);
        assert!(verify(&tree, root) > 300);
    }
    #[test]
    fn oversized_clip_paths_do_not_cache_changing_hits_past_the_layout_box() {
        let mut tree = Tree::new();
        let root = box_at(&mut tree, 0.0, 0.0, 200.0, 200.0);
        let clipper = box_at(&mut tree, 40.0, 40.0, 80.0, 80.0);
        let overflow = box_at(&mut tree, 60.0, 0.0, 160.0, 60.0);
        tree.add_child(root, clipper);
        tree.add_child(clipper, overflow);
        tree.props.insert(
            clipper,
            RenderProps {
                clip_path: Some(ClipPath::Circle {
                    radius: Some(ClipLength::Px(120.0)),
                    center: (ClipLength::Percent(50.0), ClipLength::Percent(50.0)),
                }),
                ..Default::default()
            },
        );
        layout(&mut tree, root);
        assert_eq!(test(&tree, root, 150.0, 60.0)[0].node, overflow);
        assert_ne!(
            test(&tree, root, 199.0, 60.0).first().map(|h| h.node),
            Some(overflow)
        );
        assert_eq!(test_region(&tree, root, 150.0, 60.0).1.bounds, [0.0; 4]);
        assert!(verify(&tree, root) > 0);
        tree.props.get_mut(&clipper).unwrap().clip_path = Some(ClipPath::Inset {
            top: ClipLength::Px(-20.0),
            right: ClipLength::Px(-90.0),
            bottom: ClipLength::Px(-20.0),
            left: ClipLength::Px(-20.0),
            round: Some(30.0),
        });
        assert!(verify(&tree, root) > 0);
    }
}
