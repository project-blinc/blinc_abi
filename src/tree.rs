//! Host-independent scene storage shared with the compatibility adapter.
use blinc_layout::{
    element::RenderProps,
    tree::{LayoutNodeId, LayoutTree},
};
use std::collections::{HashMap, HashSet};
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
    pub(crate) image_sources: HashMap<String, [Option<i32>; 3]>,
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
    /// Each liquid glass node's dispersion, bevel strength and curvature.
    pub(crate) glass_effects: HashMap<LayoutNodeId, crate::types::GlassEffects>,
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
        // The visible rect so far, in the root's coordinates, and where the next node's parent puts its children.
        let mut clip = [
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ];
        let mut origin = (0.0f32, 0.0f32);
        for (i, &n) in path.iter().enumerate() {
            // Under display: none, laid out as nothing at the origin, it is not drawn at all.
            if self
                .layout
                .get_style(n)
                .is_some_and(|s| s.display == taffy::Display::None)
            {
                return false;
            }
            // A fragment, as <for> and <if> make, has no box: its children are placed as its parent places them.
            let Some(layout) = self.layout.get_layout(n) else {
                if i == path.len() - 1 {
                    return true;
                }
                continue;
            };
            let mut x = origin.0 + layout.location.x;
            let mut y = origin.1 + layout.location.y;
            let (mut w, mut h) = (layout.size.width, layout.size.height);
            if let Some(v) = self.visuals.get(&n) {
                x += v[0];
                y += v[1];
                if v[2] >= 0.0 {
                    (w, h) = (v[2], v[3].max(0.0));
                }
            }
            if self.props.get(&n).is_some_and(|p| !p.visible) {
                return false;
            }
            let clips = i == 0
                || self.layout.get_style(n).is_some_and(|s| {
                    s.overflow.x != taffy::Overflow::Visible
                        || s.overflow.y != taffy::Overflow::Visible
                });
            if clips {
                clip = [
                    clip[0].max(x),
                    clip[1].max(y),
                    clip[2].min(x + w),
                    clip[3].min(y + h),
                ];
            }
            if i == path.len() - 1 {
                return x <= clip[2]
                    && x + w >= clip[0]
                    && y <= clip[3]
                    && y + h >= clip[1]
                    && clip[0] <= clip[2]
                    && clip[1] <= clip[3];
            }
            let (sx, sy) = self.scrolls.get(&n).map_or((0.0, 0.0), |s| (s.x, s.y));
            origin = (x - sx, y - sy);
        }
        true
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
            glass_effects: HashMap::new(),
            visuals: HashMap::new(),
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
        self.glass_effects.remove(&id);
        self.visuals.remove(&id);
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
