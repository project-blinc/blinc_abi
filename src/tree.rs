//! Host-independent scene storage shared with the compatibility adapter.
use crate::layout_tree::LayoutTree;
use blinc_layout::{element::RenderProps, tree::LayoutNodeId};
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
    /// Optional bitmap object mapping: fit (0 cover, 1 contain, 2 fill, 3 tile,
    /// 4 none, 5 scale-down), then x/y fractions and x/y pixel offsets.
    /// Legacy centred fits need no entry. Renderers resolve this before atlas sampling.
    pub(crate) image_mappings: HashMap<LayoutNodeId, [f32; 5]>,
    /// Background mapping is separate from a node’s content image mapping.
    pub(crate) background_mappings: HashMap<LayoutNodeId, crate::types::ImageMapping>,
    /// Image-brush sources resolved by the host, with one prepared slot per fit.
    #[cfg(feature = "scene")]
    pub(crate) image_sources: HashMap<String, [Option<i32>; 4]>,
    /// Nodes that paint with the GPU themselves in their content box, named
    /// by a slot the caller resolves while the frame is drawn.
    pub(crate) canvases: HashMap<LayoutNodeId, i32>,
    /// Scroll containers: how far their content is scrolled, and their thumb.
    pub(crate) scrolls: HashMap<LayoutNodeId, Scroll>,
    /// Only scrollbars with a non-default width or track need a style entry.
    pub(crate) scrollbar_styles: HashMap<LayoutNodeId, ScrollbarStyle>,
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
    /// Only nodes with non-default border patterns need an entry, top first, clockwise.
    pub(crate) border_styles: HashMap<LayoutNodeId, [crate::css::border::Style; 4]>,
    /// How each node decorates its text and its descendants'.
    pub(crate) text_decorations: HashMap<LayoutNodeId, crate::css::text_decoration::TextDecoration>,
    /// Each node's text shadows, first on top: its own and its descendants' text's, the nearest
    /// node with an entry winning; an empty list is `none`, which stops one inherited.
    pub(crate) text_shadows: HashMap<LayoutNodeId, Vec<blinc_core::Shadow>>,
    /// Each block container's `text-overflow`: true for an ellipsis, false for clip.
    pub(crate) text_overflows: HashMap<LayoutNodeId, bool>,
    /// Each block container's `line-clamp`: how many lines its text shows.
    pub(crate) line_clamps: HashMap<LayoutNodeId, u32>,
    /// The text nodes whose height a line clamp is holding, so it can let go of them.
    pub(crate) clamped: HashSet<LayoutNodeId>,
    /// Each liquid glass node's dispersion, bevel strength and curvature.
    pub(crate) glass_effects: HashMap<LayoutNodeId, crate::types::GlassEffects>,
    /// The insets of each `position: sticky` node, top, right, bottom and left: how far in from the
    /// edges of its scroll container it is held. Layout places it as `relative` with none of them.
    pub(crate) sticky: HashMap<LayoutNodeId, [Option<f32>; 4]>,
    pub(crate) sticky_expressions: HashMap<LayoutNodeId, [Option<crate::css::calc::Expression>; 4]>,
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

#[derive(Clone, Copy)]
pub(crate) struct ScrollbarStyle {
    pub width: f32,
    pub track: [f32; 4],
}

impl Default for ScrollbarStyle {
    fn default() -> Self {
        Self {
            width: 4.0,
            track: [0.0; 4],
        }
    }
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
    #[cfg(feature = "scene")]
    pub(crate) fn set_border_style(
        &mut self,
        node: LayoutNodeId,
        side: usize,
        style: crate::css::border::Style,
    ) {
        use crate::css::border::Style;
        let mut styles = self
            .border_styles
            .get(&node)
            .copied()
            .unwrap_or([Style::Solid; 4]);
        styles[side] = style;
        self.set_border_styles(node, styles);
    }

    #[cfg(feature = "scene")]
    pub(crate) fn set_border_styles(
        &mut self,
        node: LayoutNodeId,
        styles: [crate::css::border::Style; 4],
    ) {
        if styles == [crate::css::border::Style::Solid; 4] {
            self.border_styles.remove(&node);
        } else {
            self.border_styles.insert(node, styles);
        }
    }

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
    pub(crate) fn sticky_insets(
        &self,
        node: LayoutNodeId,
    ) -> Result<[Option<f32>; 4], &'static str> {
        let mut insets = self.sticky.get(&node).copied().ok_or("Not sticky")?;
        if let Some(expressions) = self.sticky_expressions.get(&node) {
            let basis = self
                .layout
                .ancestors(node)
                .first()
                .and_then(|p| self.layout.get_layout(*p))
                .map_or([0.0, 0.0], |l| {
                    [
                        l.size.width
                            - l.padding.left
                            - l.padding.right
                            - l.border.left
                            - l.border.right,
                        l.size.height
                            - l.padding.top
                            - l.padding.bottom
                            - l.border.top
                            - l.border.bottom,
                    ]
                });
            for (i, e) in expressions.iter().enumerate() {
                if let Some(e) = e {
                    let value =
                        e.try_resolve(basis[if i == 0 || i == 2 { 1 } else { 0 }] as f64)? as f32;
                    if !value.is_finite() {
                        return Err("Non-finite sticky CSS length");
                    }
                    insets[i] = Some(value);
                }
            }
        }
        Ok(insets)
    }

    /// How far a `position: sticky` node is drawn from where layout puts it: held `top`, `right`,
    /// `bottom` and `left` in from the edges of the view of its nearest scroll container, and
    /// never out of its parent. Zero for any other node, and one with nothing to scroll in.
    pub(crate) fn sticky_shift(&self, node: LayoutNodeId) -> (f32, f32) {
        let Ok(insets) = self.sticky_insets(node) else {
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

    /// A row that creates a stacking context keeps its background and contents together.
    /// Ordinary row backgrounds precede all the shared grid's cell contents.
    #[cfg(feature = "scene")]
    pub(crate) fn row_stacks(&self, node: LayoutNodeId) -> bool {
        self.z_index.contains_key(&node)
            || self.sticky.contains_key(&node)
            || self.props.get(&node).is_some_and(|p| {
                p.opacity < 1.0
                    || p.transform.is_some()
                    || p.filter.is_some()
                    || p.mask_image.is_some()
                    || p.clip_path.is_some()
            })
    }

    /// Apply a position/inset batch to the sticky constraints shared by every adapter.
    /// Insets hold a sticky box at the scroll edge; its normal-flow layout has none.
    pub(crate) fn sync_sticky(
        &mut self,
        node: LayoutNodeId,
        style: &mut taffy::Style,
        position: Option<bool>,
        writes: [Option<Option<f32>>; 4],
    ) {
        use taffy::style::{CompactLength, LengthPercentageAuto};
        let held = self.sticky.get(&node).copied();
        let sticky = position.unwrap_or(held.is_some());
        if !sticky && held.is_none() {
            return;
        }
        let mut insets = held.unwrap_or_else(|| {
            [
                style.inset.top,
                style.inset.right,
                style.inset.bottom,
                style.inset.left,
            ]
            .map(|v| {
                let c = v.into_raw();
                (c.tag() == CompactLength::LENGTH_TAG).then_some(c.value())
            })
        });
        let mut expressions = self
            .sticky_expressions
            .remove(&node)
            .unwrap_or_else(|| std::array::from_fn(|_| None));
        for (i, value) in [
            style.inset.top,
            style.inset.right,
            style.inset.bottom,
            style.inset.left,
        ]
        .iter()
        .enumerate()
        {
            let raw = value.into_raw();
            if raw.is_calc() {
                expressions[i] = self
                    .layout
                    .expressions
                    .get(&(raw.calc_value() as usize))
                    .cloned();
            } else if raw.tag() == CompactLength::PERCENT_TAG {
                expressions[i] = Some(crate::css::calc::Expression::Linear(
                    crate::css::calc::Length {
                        px: 0.0,
                        percent: raw.value() as f64,
                    },
                ));
            }
            if writes[i].is_some() {
                expressions[i] = None;
            }
        }
        for (slot, write) in insets.iter_mut().zip(writes) {
            if let Some(v) = write {
                *slot = v;
            }
        }
        let inset = if sticky {
            self.sticky.insert(node, insets);
            [LengthPercentageAuto::auto(); 4]
        } else {
            self.sticky.remove(&node);
            insets.map(|v| v.map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length))
        };
        [
            style.inset.top,
            style.inset.right,
            style.inset.bottom,
            style.inset.left,
        ] = inset;
        if sticky {
            if expressions.iter().any(Option::is_some) {
                self.sticky_expressions.insert(node, expressions);
            }
        } else {
            for (i, e) in expressions.into_iter().enumerate() {
                if let Some(e) = e {
                    let handle = crate::layout_tree::new_handle();
                    self.layout.expressions.insert(handle, e);
                    crate::layout_tree::set_math(style, 30 + i as i32, handle);
                }
            }
        }
    }

    /// Hold the text under each line-clamped container to that many lines: a text node's height is
    /// capped at its lines' pitch times the clamp, and let go again when the clamp is. Before layout.
    pub(crate) fn sync_line_clamps(&mut self) {
        if self.line_clamps.is_empty() && self.clamped.is_empty() {
            return;
        }
        let mut want: HashMap<LayoutNodeId, f32> = HashMap::new();
        for (&container, &lines) in &self.line_clamps {
            if lines == 0 {
                continue;
            }
            let mut stack = vec![container];
            while let Some(n) = stack.pop() {
                // A container of its own clamps what it holds.
                if n != container && self.line_clamps.contains_key(&n) {
                    continue;
                }
                if let Some(context) = self.layout.text_context(n) {
                    want.insert(n, lines as f32 * context.font_size * context.line_height);
                } else {
                    stack.extend(self.layout.children(n));
                }
            }
        }
        let set = |tree: &mut Tree, n: LayoutNodeId, height: taffy::LengthPercentageAuto| {
            if let Some(mut style) = tree.layout.get_style(n)
                && style.max_size.height != height
            {
                style.max_size.height = height;
                tree.layout.set_style(n, style);
            }
        };
        for (&n, &h) in &want {
            set(self, n, taffy::LengthPercentageAuto::length(h));
        }
        for n in std::mem::take(&mut self.clamped) {
            if !want.contains_key(&n) {
                set(self, n, taffy::LengthPercentageAuto::auto());
            }
        }
        self.clamped = want.into_keys().collect();
    }

    pub fn new() -> Self {
        Self {
            layout: LayoutTree::new(),
            props: HashMap::new(),
            #[cfg(feature = "hashlink")]
            pruned: false,
            owners: HashMap::new(),
            images: HashMap::new(),
            image_mappings: HashMap::new(),
            background_mappings: HashMap::new(),
            #[cfg(feature = "scene")]
            image_sources: HashMap::new(),
            canvases: HashMap::new(),
            scrolls: HashMap::new(),
            scrollbar_styles: HashMap::new(),
            pass_through: HashSet::new(),
            notches: HashMap::new(),
            #[cfg(feature = "scene")]
            even_odd: HashSet::new(),
            backdrop_filters: HashMap::new(),
            border_styles: HashMap::new(),
            text_decorations: HashMap::new(),
            text_shadows: HashMap::new(),
            text_overflows: HashMap::new(),
            line_clamps: HashMap::new(),
            clamped: HashSet::new(),
            image_uses: HashMap::new(),
            glass_effects: HashMap::new(),
            visuals: HashMap::new(),
            sticky: HashMap::new(),
            sticky_expressions: HashMap::new(),
            z_index: HashMap::new(),
        }
    }
    /// Sets bitmap object mapping without allocating for the default centred fits.
    /// A negative fit resets it. Nonfinite positions and unknown fits are rejected.
    pub fn set_image_mapping(&mut self, node: LayoutNodeId, fit: i32, position: [f32; 4]) {
        if fit < 0 {
            self.image_mappings.remove(&node);
        } else if fit <= 5 && position.iter().all(|v| v.is_finite()) {
            if fit <= 3 && position == [0.5, 0.5, 0.0, 0.0] {
                self.image_mappings.remove(&node);
            } else {
                self.image_mappings.insert(
                    node,
                    [
                        fit as f32,
                        position[0],
                        position[1],
                        position[2],
                        position[3],
                    ],
                );
            }
        }
    }

    pub(crate) fn forget(&mut self, id: LayoutNodeId) {
        self.props.remove(&id);
        self.owners.remove(&id);
        self.images.remove(&id);
        self.image_mappings.remove(&id);
        self.background_mappings.remove(&id);
        self.canvases.remove(&id);
        self.scrolls.remove(&id);
        self.scrollbar_styles.remove(&id);
        self.pass_through.remove(&id);
        self.notches.remove(&id);
        self.backdrop_filters.remove(&id);
        self.border_styles.remove(&id);
        self.text_decorations.remove(&id);
        self.text_shadows.remove(&id);
        self.text_overflows.remove(&id);
        self.line_clamps.remove(&id);
        self.clamped.remove(&id);
        self.image_uses.remove(&id);
        self.glass_effects.remove(&id);
        self.visuals.remove(&id);
        self.sticky.remove(&id);
        self.sticky_expressions.remove(&id);
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

#[cfg(test)]
mod image_mapping_tests {
    use super::*;

    #[test]
    fn optional_mapping_keeps_fractions_resets_and_does_not_retain_removed_nodes() {
        let mut tree = Tree::new();
        let node = LayoutNodeId::from_raw(42);
        tree.set_image_mapping(node, 2, [0.5, 0.5, 0.0, 0.0]);
        assert!(tree.image_mappings.is_empty());
        tree.set_image_mapping(node, 5, [0.125, 0.875, -0.375, 1.625]);
        assert_eq!(
            tree.image_mappings[&node],
            [5.0, 0.125, 0.875, -0.375, 1.625]
        );
        tree.set_image_mapping(node, 6, [0.0; 4]);
        tree.set_image_mapping(node, 1, [f32::NAN; 4]);
        assert_eq!(tree.image_mappings[&node][0], 5.0);
        tree.set_image_mapping(node, -1, [0.0; 4]);
        assert!(tree.image_mappings.is_empty());
        tree.set_image_mapping(node, 4, [0.5, 0.5, 0.0, 0.0]);
        tree.forget(node);
        assert!(tree.image_mappings.is_empty());
    }
}
