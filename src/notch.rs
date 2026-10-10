//! A notch's outline as a signed distance, on the CPU: the shaders' `sdNotch`,
//! for hit-testing a point against what is drawn. A notch is three rows: the
//! four signed corner radii (negative for a concave corner that flares out
//! to the box's edge), then the top and bottom edges' modifiers, each as
//! kind (0 none, 1 scoop, 2 bulge, 3 cut, 4 peak), width, height and radius.

type V2 = (f32, f32);

fn len(v: V2) -> f32 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}

/// Distance to a box at `origin` of `size` with round corners of `radius`:
/// top-left, top-right, bottom-right, bottom-left.
fn rounded_rect(p: V2, origin: V2, size: V2, radius: [f32; 4]) -> f32 {
    let half = (size.0 * 0.5, size.1 * 0.5);
    let rel = (p.0 - origin.0 - half.0, p.1 - origin.1 - half.1);
    let q = (rel.0.abs() - half.0, rel.1.abs() - half.1);
    let r = match (rel.0 > 0.0, rel.1 < 0.0) {
        (false, true) => radius[0],
        (true, true) => radius[1],
        (true, false) => radius[2],
        (false, false) => radius[3],
    }
    .min(half.0.min(half.1))
    .max(0.0);
    let qa = (q.0 + r, q.1 + r);
    len((qa.0.max(0.0), qa.1.max(0.0))) + qa.0.max(qa.1).min(0.0) - r
}

/// Distance to the ellipse about `c` of `radii`, scaled by its smaller radius.
fn ellipse_at(p: V2, c: V2, radii: V2) -> f32 {
    let (rx, ry) = (radii.0.max(0.001), radii.1.max(0.001));
    (len(((p.0 - c.0) / rx, (p.1 - c.1) / ry)) - 1.0) * radii.0.min(radii.1)
}

/// Distance to triangle `abc`.
fn triangle(p: V2, a: V2, b: V2, c: V2) -> f32 {
    let sub = |x: V2, y: V2| (x.0 - y.0, x.1 - y.1);
    let dot = |x: V2, y: V2| x.0 * y.0 + x.1 * y.1;
    let (e0, e1, e2) = (sub(b, a), sub(c, b), sub(a, c));
    let (v0, v1, v2) = (sub(p, a), sub(p, b), sub(p, c));
    let near = |v: V2, e: V2| {
        let t = (dot(v, e) / dot(e, e).max(1e-6)).clamp(0.0, 1.0);
        (v.0 - e.0 * t, v.1 - e.1 * t)
    };
    let (pq0, pq1, pq2) = (near(v0, e0), near(v1, e1), near(v2, e2));
    let s = (e0.0 * e2.1 - e0.1 * e2.0).signum();
    let cross = |v: V2, e: V2| s * (v.0 * e.1 - v.1 * e.0);
    let pairs = [
        (dot(pq0, pq0), cross(v0, e0)),
        (dot(pq1, pq1), cross(v1, e1)),
        (dot(pq2, pq2), cross(v2, e2)),
    ];
    let d2 = pairs.iter().map(|p| p.0).fold(f32::MAX, f32::min);
    let side = pairs.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    -d2.sqrt() * side.signum()
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}

/// Whether `notch` draws anything other than the plain box.
pub fn is_notch(notch: &[[f32; 4]; 3]) -> bool {
    notch[0].iter().map(|r| r.abs()).sum::<f32>() + notch[1][0] + notch[2][0] > 0.0
}

/// Signed distance from `p` to the notch filling `size` from the origin:
/// negative inside what is drawn.
pub fn distance(p: V2, size: V2, notch: &[[f32; 4]; 3]) -> f32 {
    let [corners, top, bottom] = *notch;
    let r = corners.map(f32::abs);
    let concave = corners.map(|c| c < 0.0);
    let protrudes = |m: [f32; 4]| m[0] > 1.5 && m[0] < 2.5 || m[0] > 3.5 && m[0] < 4.5;
    let top_h = if protrudes(top) { top[2] } else { 0.0 };
    let bottom_h = if protrudes(bottom) { bottom[2] } else { 0.0 };
    let c = |i: usize| if concave[i] { r[i] } else { 0.0 };
    let (tl, tr, br, bl) = (c(0), c(1), c(2), c(3));
    let left = tl.max(bl);
    let right = tr.max(br);
    let top_offset = tl.max(tr).max(top_h);
    let bottom_offset = bl.max(br).max(bottom_h);
    let inner_origin = (left, top_offset);
    let inner_size = (
        (size.0 - left - right).max(0.001),
        (size.1 - top_offset - bottom_offset).max(0.001),
    );
    // Concave corners are square on the body; their curve is the flare.
    let inner_radii = [0, 1, 2, 3].map(|i| if concave[i] { 0.0 } else { r[i] });
    let mut d = rounded_rect(p, inner_origin, inner_size, inner_radii);
    let inner_right = inner_origin.0 + inner_size.0;
    let inner_bottom = inner_origin.1 + inner_size.1;
    let into = inner_size.0.min(inner_size.1) * 0.5;
    let flare_into = into.min(8.0);
    let room = (size.1 - top_offset - bottom_offset).max(0.0);
    let sharp = [0.0; 4];
    if concave[0] {
        let ry = r[0].min(room);
        let box_d = rounded_rect(p, (0.0, inner_origin.1), (left + flare_into, ry), sharp);
        d = d.min(box_d.max(-ellipse_at(p, (0.0, inner_origin.1 + ry), (left, ry))));
    }
    if concave[1] {
        let ry = r[1].min(room);
        let w = size.0 - inner_right;
        let box_d = rounded_rect(
            p,
            (inner_right - flare_into, inner_origin.1),
            (w + flare_into, ry),
            sharp,
        );
        d = d.min(box_d.max(-ellipse_at(p, (size.0, inner_origin.1 + ry), (w, ry))));
    }
    if concave[2] {
        let ry = r[2].min(room);
        let w = size.0 - inner_right;
        let box_d = rounded_rect(
            p,
            (inner_right - flare_into, inner_bottom - ry),
            (w + flare_into, ry),
            sharp,
        );
        d = d.min(box_d.max(-ellipse_at(p, (size.0, inner_bottom - ry), (w, ry))));
    }
    if concave[3] {
        let ry = r[3].min(room);
        let box_d = rounded_rect(p, (0.0, inner_bottom - ry), (left + flare_into, ry), sharp);
        d = d.min(box_d.max(-ellipse_at(p, (0.0, inner_bottom - ry), (left, ry))));
    }
    d = edge(p, d, size.0 * 0.5, inner_origin.1, top, 1.0, into);
    edge(p, d, size.0 * 0.5, inner_bottom, bottom, -1.0, into)
}

/// `d` with edge modifier `m` at the centre `cx` of the edge at `base_y`;
/// `dir` 1 on the top edge and -1 on the bottom, the body lying the way it points.
fn edge(p: V2, d: f32, cx: f32, base_y: f32, m: [f32; 4], dir: f32, into: f32) -> f32 {
    let (kind, w, h) = (m[0], m[1], m[2]);
    if kind <= 0.5 || w <= 0.001 || h <= 0.001 {
        return d;
    }
    let half_w = w * 0.5;
    // In the edge's frame: y grows into the body from the baseline.
    let q = (p.0, (p.1 - base_y) * dir);
    if kind < 1.5 {
        let disk_r = half_w.min(h);
        let disk_y = h - disk_r;
        let disk = (len((q.0 - cx, q.1 - disk_y)) - disk_r).max(disk_y - q.1);
        let hollow = if disk_y > 0.001 {
            rounded_rect(q, (cx - half_w, 0.0), (w, disk_y), [0.0; 4]).min(disk)
        } else {
            disk
        };
        smax(d, -hollow, m[3].max(0.001))
    } else if kind < 2.5 {
        let rb = (half_w * half_w + h * h) / (2.0 * h).max(0.001);
        let side = if q.1 < 0.0 {
            -100000.0
        } else {
            (q.0 - cx).abs() - half_w
        };
        let cap = (len((q.0 - cx, q.1 - (rb - h))) - rb)
            .max(q.1 - into)
            .max(side);
        smin(d, cap, m[3].max(0.001))
    } else if kind < 3.5 {
        smax(
            d,
            -triangle(q, (cx - half_w, 0.0), (cx, h), (cx + half_w, 0.0)),
            1.5,
        )
    } else {
        let spread = half_w * (h + into) / h;
        let peak = triangle(q, (cx - spread, into), (cx, -h), (cx + spread, into))
            .max((q.0 - cx).abs() - half_w);
        smin(d, peak, 1.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(p: V2, size: V2, notch: &[[f32; 4]; 3]) -> bool {
        distance(p, size, notch) < 0.0
    }

    #[test]
    fn a_concave_top_flares_out_and_its_body_is_inset() {
        // A dropdown: concave top corners of 20, round bottom ones of 10, in 200 by 100.
        let n = [[-20.0, -20.0, 10.0, 10.0], [0.0; 4], [0.0; 4]];
        assert!(is_notch(&n));
        assert!(inside((100.0, 50.0), (200.0, 100.0), &n));
        // The body is inset by the concave radius: nothing above it.
        assert!(!inside((100.0, 10.0), (200.0, 100.0), &n));
        // The flare's foot along the body's top is drawn; below it, beside the body, is the cut-out.
        assert!(inside((10.0, 21.0), (200.0, 100.0), &n));
        assert!(inside((18.0, 30.0), (200.0, 100.0), &n));
        assert!(!inside((2.0, 30.0), (200.0, 100.0), &n));
        assert!(!inside((2.0, 60.0), (200.0, 100.0), &n));
        assert!(inside((25.0, 60.0), (200.0, 100.0), &n));
    }

    #[test]
    fn edge_modifiers_cut_in_and_add_on() {
        let size = (200.0, 100.0);
        let round = [8.0, 8.0, 8.0, 8.0];
        // A scoop 40 wide and 12 deep at the top's centre: empty in its bowl, filled beside it.
        let scoop = [round, [1.0, 40.0, 12.0, 4.0], [0.0; 4]];
        assert!(!inside((100.0, 4.0), size, &scoop));
        assert!(inside((60.0, 4.0), size, &scoop));
        // A peak 20 wide rising 10: the body is inset by it, and the peak is drawn above it.
        let peak = [round, [4.0, 20.0, 10.0, 0.0], [0.0; 4]];
        assert!(inside((100.0, 4.0), size, &peak));
        assert!(!inside((60.0, 4.0), size, &peak));
        // A V cut 30 wide and 12 deep at the bottom.
        let cut = [round, [0.0; 4], [3.0, 30.0, 12.0, 0.0]];
        assert!(!inside((100.0, 96.0), size, &cut));
        assert!(inside((60.0, 96.0), size, &cut));
        // A plain box is no notch.
        assert!(!is_notch(&[[0.0; 4]; 3]));
    }
}
