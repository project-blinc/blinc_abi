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
    /// Nodes that paint with the GPU themselves in their content box, named
    /// by a slot the caller resolves while the frame is drawn.
    pub(crate) canvases: HashMap<LayoutNodeId, i32>,
    /// Scroll containers: how far their content is scrolled, and their thumb.
    pub(crate) scrolls: HashMap<LayoutNodeId, Scroll>,
    /// Nodes the hit test passes through, with everything inside them, as CSS's `pointer-events: none`.
    pub(crate) pass_through: std::collections::HashSet<LayoutNodeId>,
    /// Nodes drawn as a notch: signed corner radii, then the top and bottom edges' modifiers.
    pub(crate) notches: HashMap<LayoutNodeId, [[f32; 4]; 3]>,
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
    pub fn new() -> Self {
        Self {
            layout: LayoutTree::new(),
            props: HashMap::new(),
            #[cfg(feature = "hashlink")]
            pruned: false,
            owners: HashMap::new(),
            images: HashMap::new(),
            canvases: HashMap::new(),
            scrolls: HashMap::new(),
            pass_through: HashSet::new(),
            notches: HashMap::new(),
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
