//! Host-independent scene storage shared with the compatibility adapter.
use blinc_layout::{
    element::RenderProps,
    tree::{LayoutNodeId, LayoutTree},
};
use std::collections::{HashMap, HashSet};
/// What the paint walk knows on reaching a node's children: the visible rect
/// so far, in the root's coordinates, and where the node puts its children.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reach {
    pub(crate) clip: [f32; 4],
    pub(crate) origin: (f32, f32),
}

impl Reach {
    pub(crate) const ROOT: Reach = Reach {
        clip: [
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ],
        origin: (0.0, 0.0),
    };
}

/// A node on the paint walk.
pub(crate) enum Step {
    /// Not drawn, nor anything under it.
    Hidden,
    /// No box of its own: its children are reached as it was.
    Through,
    /// Its box: what it leaves its children, and whether any of it is in view.
    Box { reach: Reach, seen: bool },
}

pub struct Tree {
    pub(crate) layout: LayoutTree,
    /// Visual properties per node; `LayoutTree` holds only styles.
    pub(crate) props: HashMap<LayoutNodeId, RenderProps>,
    /// Set by removals, so the next flush drops props of nodes now gone.
    #[cfg(feature = "hashlink")]
    pub(crate) pruned: bool,
    /// Every live node and the handle that created it.
    pub(crate) owners: HashMap<LayoutNodeId, u64>,
    /// Nodes that draw an image in their content box: an SVG or a bitmap,
    /// named by a slot the caller resolves after the walk.
    pub(crate) images: HashMap<LayoutNodeId, i32>,
    /// Image-brush sources resolved by the host, with one prepared slot per fit.
    #[cfg(feature = "scene")]
    pub(crate) image_sources: HashMap<String, [Option<i32>; 4]>,
    /// Nodes that paint with the GPU themselves in their content box, named
    /// by a slot the caller resolves while the frame is drawn.
    pub(crate) canvases: HashMap<LayoutNodeId, i32>,
    /// Scroll containers: how far their content is scrolled, and their thumb.
    pub(crate) scrolls: HashMap<LayoutNodeId, Scroll>,
    /// Nodes the hit test passes through, with everything inside them, as CSS's `pointer-events: none`.
    pub(crate) pass_through: std::collections::HashSet<LayoutNodeId>,
    /// Nodes drawn as a notch: signed corner radii, then the top and bottom edges' modifiers.
    pub(crate) notches: HashMap<LayoutNodeId, [[f32; 4]; 3]>,
    /// Nodes whose clip-path polygon or path fills by the even-odd rule rather than nonzero.
    #[cfg(feature = "scene")]
    pub(crate) even_odd: HashSet<LayoutNodeId>,
    /// Each node's backdrop colour filters, in `BACKDROP_IDENTITY`'s order; none for the identity.
    pub(crate) backdrop_filters: HashMap<LayoutNodeId, [f32; 7]>,
    /// Nodes whose background image source changed since the host last took them, each with the
    /// source it names now, none when it names none: one entry per node, the last change.
    pub(crate) image_uses: HashMap<LayoutNodeId, Option<String>>,
    /// How each node decorates its text and its descendants'.
    pub(crate) text_decorations: HashMap<LayoutNodeId, crate::css::text_decoration::TextDecoration>,
    /// Each node's text shadows, first on top: its own and its descendants' text's, the nearest
    /// node with an entry winning; an empty list is `none`, which stops one inherited.
    pub(crate) text_shadows: HashMap<LayoutNodeId, Vec<blinc_core::Shadow>>,
    /// Each liquid glass node's dispersion, bevel strength and curvature.
    pub(crate) glass_effects: HashMap<LayoutNodeId, crate::types::GlassEffects>,
    /// The insets of each `position: sticky` node, top, right, bottom and left: how far in from the
    /// edges of its scroll container it is held. Layout places it as `relative` with none of them.
    pub(crate) sticky: HashMap<LayoutNodeId, [Option<f32>; 4]>,
    /// Each node's `z-index`, where it is not `auto`.
    pub(crate) z_index: HashMap<LayoutNodeId, i32>,
    /// Nodes drawn away from their layout while a layout animation runs:
    /// moved by (dx, dy), and at size (w, h) when w is not negative.
    pub(crate) visuals: HashMap<LayoutNodeId, [f32; 4]>,
}

/// A scroll container's state, which the paint walk and the hit test read.
#[derive(Clone, Copy, Default)]
pub struct Scroll {
    /// How far the content is moved up and left, in layout units.
    pub x: f32,
    pub y: f32,
    /// The thumb's colour, straight alpha; transparent hides it.
    pub thumb: [f32; 4],
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}
impl Tree {
    /// Whether any of `node`'s box is on screen: inside the root and inside
    /// every ancestor that clips, each scrolled and moved by its layout
    /// animation as the paint walk places it. Transforms are left out: they
    /// turn, scale or nudge a box about where layout put it, and an animated
    /// one would otherwise keep its own node in view. A fragment on the way,
    /// which has no box, is passed through; a node not laid out yet counts as
    /// in view, and a hidden or `display: none` ancestor as not. A box of no
    /// size counts where it stands, so one growing from nothing is seen.
    pub fn in_view(&self, node: LayoutNodeId) -> bool {
        let mut path = self.layout.ancestors(node);
        path.reverse();
        path.push(node);
        let mut reach = Reach::ROOT;
        for (i, &n) in path.iter().enumerate() {
            let last = i == path.len() - 1;
            match self.step(n, reach, i == 0) {
                Step::Hidden => return false,
                Step::Through if last => return true,
                Step::Through => {}
                Step::Box { seen, .. } if last => return seen,
                Step::Box { reach: next, .. } => reach = next,
            }
        }
        true
    }

    /// One node on the paint walk down to a node asked about: `reach` is what
    /// its parent left it, and `root` is whether it is the first on the path,
    /// which clips to its own box.
    pub(crate) fn step(&self, n: LayoutNodeId, reach: Reach, root: bool) -> Step {
        // Under display: none, laid out as nothing at the origin, it is not drawn at all.
        if self
            .layout
            .get_style(n)
            .is_some_and(|s| s.display == taffy::Display::None)
        {
            return Step::Hidden;
        }
        // A fragment, as <for> and <if> make, has no box: its children are placed as its parent places them.
        let Some(layout) = self.layout.get_layout(n) else {
            return Step::Through;
        };
        let (stuck_x, stuck_y) = self.sticky_shift(n);
        let mut x = reach.origin.0 + layout.location.x + stuck_x;
        let mut y = reach.origin.1 + layout.location.y + stuck_y;
        let (mut w, mut h) = (layout.size.width, layout.size.height);
        if let Some(v) = self.visuals.get(&n) {
            x += v[0];
            y += v[1];
            if v[2] >= 0.0 {
                (w, h) = (v[2], v[3].max(0.0));
            }
        }
        if self.props.get(&n).is_some_and(|p| !p.visible) {
            return Step::Hidden;
        }
        let clips = root
            || self.layout.get_style(n).is_some_and(|s| {
                s.overflow.x != taffy::Overflow::Visible || s.overflow.y != taffy::Overflow::Visible
            });
        let mut clip = reach.clip;
        if clips {
            clip = [
                clip[0].max(x),
                clip[1].max(y),
                clip[2].min(x + w),
                clip[3].min(y + h),
            ];
        }
        let seen = x <= clip[2]
            && x + w >= clip[0]
            && y <= clip[3]
            && y + h >= clip[1]
            && clip[0] <= clip[2]
            && clip[1] <= clip[3];
        let (sx, sy) = self.scrolls.get(&n).map_or((0.0, 0.0), |s| (s.x, s.y));
        Step::Box {
            reach: Reach {
                clip,
                origin: (x - sx, y - sy),
            },
            seen,
        }
    }
    /// How far a `position: sticky` node is drawn from where layout puts it: held `top`, `right`,
    /// `bottom` and `left` in from the edges of the view of its nearest scroll container, and
    /// never out of its parent. Zero for any other node, and one with nothing to scroll in.
    pub(crate) fn sticky_shift(&self, node: LayoutNodeId) -> (f32, f32) {
        let Some(insets) = self.sticky.get(&node) else {
            return (0.0, 0.0);
        };
        let Some(own) = self.layout.get_layout(node) else {
            return (0.0, 0.0);
        };
        let overflows = |n: LayoutNodeId| {
            self.layout.get_style(n).is_some_and(|s| {
                s.overflow.x != taffy::Overflow::Visible || s.overflow.y != taffy::Overflow::Visible
            })
        };
        // Where it is in the content of the scroll container, which the container's scroll moves.
        let (mut cx, mut cy) = (own.location.x, own.location.y);
        let (w, h) = (own.size.width, own.size.height);
        let mut parent: Option<(f32, f32, f32, f32)> = None;
        let mut container = None;
        let mut below = Vec::new();
        for a in self.layout.ancestors(node) {
            if overflows(a) {
                container = Some(a);
                break;
            }
            below.push(a);
        }
        let Some(container) = container else {
            return (0.0, 0.0);
        };
        let Some(view) = self.layout.get_layout(container) else {
            return (0.0, 0.0);
        };
        // The parent's box in the same space, and the node's place in it.
        let mut offset = (0.0, 0.0);
        for (i, a) in below.iter().enumerate() {
            let Some(l) = self.layout.get_layout(*a) else {
                return (0.0, 0.0);
            };
            if i == 0 {
                parent = Some((0.0, 0.0, l.size.width, l.size.height));
                offset = (l.location.x, l.location.y);
            } else {
                offset.0 += l.location.x;
                offset.1 += l.location.y;
            }
            cx += l.location.x;
            cy += l.location.y;
        }
        // A parent that is the scroll container itself holds the node within its whole content.
        let (px, py, pw, ph) = match parent {
            Some((_, _, pw, ph)) => (offset.0, offset.1, pw, ph),
            None => (
                0.0,
                0.0,
                view.scrollable_overflow_rect.right.max(view.size.width),
                view.scrollable_overflow_rect.bottom.max(view.size.height),
            ),
        };
        let (sx, sy) = self
            .scrolls
            .get(&container)
            .map_or((0.0, 0.0), |s| (s.x, s.y));
        let shift = |place: f32,
                     size: f32,
                     scroll: f32,
                     view: f32,
                     lo: Option<f32>,
                     hi: Option<f32>,
                     p0: f32,
                     p1: f32| {
            let mut d = 0.0f32;
            if let Some(lo) = lo {
                d = d.max(scroll + lo - place);
            }
            if let Some(hi) = hi {
                d = d.min(scroll + view - hi - (place + size));
            }
            // Not out of its parent.
            d.min(p1 - (place + size)).max(p0 - place)
        };
        let dy = shift(
            cy,
            h,
            sy,
            view.size.height,
            insets[0],
            insets[2],
            py,
            py + ph,
        );
        let dx = shift(
            cx,
            w,
            sx,
            view.size.width,
            insets[3],
            insets[1],
            px,
            px + pw,
        );
        (
            if dx.is_finite() { dx } else { 0.0 },
            if dy.is_finite() { dy } else { 0.0 },
        )
    }

    /// The children of `node` in the order they are painted, the last on top, which is the order
    /// they are hit in reverse. As CSS stacks a context's children: those with a negative
    /// `z-index`, then the rest in tree order, then those with `z-index: 0` and sticky ones in
    /// tree order, then those with a positive `z-index`; ties in tree order. Each node stacks
    /// only among its siblings, as if it began a stacking context. Nodes with no `z-index`, bar
    /// sticky ones, keep their tree order, as they did before `z-index` was read.
    pub(crate) fn paint_order(&self, node: LayoutNodeId) -> Vec<LayoutNodeId> {
        let mut children = self.layout.children(node);
        if self.z_index.is_empty() && self.sticky.is_empty() {
            return children;
        }
        // Which of four layers, then by z-index within the outer two.
        let layer = |n: &LayoutNodeId| -> (u8, i32) {
            match self.z_index.get(n) {
                Some(z) if *z < 0 => (0, *z),
                Some(0) => (2, 0),
                Some(z) => (3, *z),
                None if self.sticky.contains_key(n) => (2, 0),
                None => (1, 0),
            }
        };
        children.sort_by_key(layer);
        children
    }

    pub fn new() -> Self {
        Self {
            layout: LayoutTree::new(),
            props: HashMap::new(),
            #[cfg(feature = "hashlink")]
            pruned: false,
            owners: HashMap::new(),
            images: HashMap::new(),
            #[cfg(feature = "scene")]
            image_sources: HashMap::new(),
            canvases: HashMap::new(),
            scrolls: HashMap::new(),
            pass_through: HashSet::new(),
            notches: HashMap::new(),
            #[cfg(feature = "scene")]
            even_odd: HashSet::new(),
            backdrop_filters: HashMap::new(),
            text_decorations: HashMap::new(),
            text_shadows: HashMap::new(),
            image_uses: HashMap::new(),
            glass_effects: HashMap::new(),
            visuals: HashMap::new(),
            sticky: HashMap::new(),
            z_index: HashMap::new(),
        }
    }
    pub(crate) fn forget(&mut self, id: LayoutNodeId) {
        self.props.remove(&id);
        self.owners.remove(&id);
        self.images.remove(&id);
        self.canvases.remove(&id);
        self.scrolls.remove(&id);
        self.pass_through.remove(&id);
        self.notches.remove(&id);
        self.backdrop_filters.remove(&id);
        self.text_decorations.remove(&id);
        self.text_shadows.remove(&id);
        self.image_uses.remove(&id);
        self.glass_effects.remove(&id);
        self.visuals.remove(&id);
        self.sticky.remove(&id);
        self.z_index.remove(&id);
    }
}
impl std::ops::Deref for Tree {
    type Target = LayoutTree;
    fn deref(&self) -> &LayoutTree {
        &self.layout
    }
}
impl std::ops::DerefMut for Tree {
    fn deref_mut(&mut self) -> &mut LayoutTree {
        &mut self.layout
    }
}
