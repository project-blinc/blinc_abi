//! Native layout with CSS math resolved by Taffy at each property's percentage basis.
//!
//! Ordinary trees retain Blinc's cached layout path. Trees with deferred lengths use the same
//! Taffy algorithms over an ABI-owned view; opaque handles are looked up, never dereferenced.
use crate::css::calc::Expression;
use blinc_layout::{
    element::ElementBounds,
    text_measure::{TextLayoutOptions, measure_text_with_options},
    tree::{LayoutNodeId, LayoutTree as BaseTree, TextMeasureContext},
};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};
use taffy::*;

static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(8);
pub(crate) fn new_handle() -> usize {
    NEXT_HANDLE.fetch_add(8, Ordering::Relaxed)
}

pub(crate) fn set_math(style: &mut Style, raw: i32, handle: usize) -> bool {
    let ptr = handle as *const ();
    let dimension = Dimension::calc(ptr);
    let auto = LengthPercentageAuto::calc(ptr);
    let length = LengthPercentage::calc(ptr);
    match raw {
        10 => style.size.width = dimension,
        11 => style.size.height = dimension,
        12 => style.min_size.width = auto,
        13 => style.max_size.width = auto,
        14 => style.min_size.height = auto,
        15 => style.max_size.height = auto,
        26 => style.flex_basis = dimension,
        30 => style.inset.top = auto,
        31 => style.inset.right = auto,
        32 => style.inset.bottom = auto,
        33 => style.inset.left = auto,
        43 => style.padding.top = length,
        44 => style.padding.right = length,
        45 => style.padding.bottom = length,
        46 => style.padding.left = length,
        47 => style.margin.top = auto,
        48 => style.margin.right = auto,
        49 => style.margin.bottom = auto,
        50 => style.margin.left = auto,
        51 => style.gap.width = length,
        52 => style.gap.height = length,
        _ => return false,
    }
    true
}

pub(crate) fn math_handle(style: &Style, raw: i32) -> Option<usize> {
    let value = match raw {
        10 => style.size.width.into_raw(),
        11 => style.size.height.into_raw(),
        12 => style.min_size.width.into_raw(),
        13 => style.max_size.width.into_raw(),
        14 => style.min_size.height.into_raw(),
        15 => style.max_size.height.into_raw(),
        26 => style.flex_basis.into_raw(),
        30 => style.inset.top.into_raw(),
        31 => style.inset.right.into_raw(),
        32 => style.inset.bottom.into_raw(),
        33 => style.inset.left.into_raw(),
        43 => style.padding.top.into_raw(),
        44 => style.padding.right.into_raw(),
        45 => style.padding.bottom.into_raw(),
        46 => style.padding.left.into_raw(),
        47 => style.margin.top.into_raw(),
        48 => style.margin.right.into_raw(),
        49 => style.margin.bottom.into_raw(),
        50 => style.margin.left.into_raw(),
        51 => style.gap.width.into_raw(),
        52 => style.gap.height.into_raw(),
        _ => return None,
    };
    value.is_calc().then(|| value.calc_value() as usize)
}

fn handles(style: &Style) -> impl Iterator<Item = usize> {
    let values = [
        style.size.width.into_raw(),
        style.size.height.into_raw(),
        style.min_size.width.into_raw(),
        style.max_size.width.into_raw(),
        style.min_size.height.into_raw(),
        style.max_size.height.into_raw(),
        style.flex_basis.into_raw(),
        style.inset.top.into_raw(),
        style.inset.right.into_raw(),
        style.inset.bottom.into_raw(),
        style.inset.left.into_raw(),
        style.padding.top.into_raw(),
        style.padding.right.into_raw(),
        style.padding.bottom.into_raw(),
        style.padding.left.into_raw(),
        style.margin.top.into_raw(),
        style.margin.right.into_raw(),
        style.margin.bottom.into_raw(),
        style.margin.left.into_raw(),
        style.gap.width.into_raw(),
        style.gap.height.into_raw(),
    ];
    values
        .into_iter()
        .filter(|v| v.is_calc())
        .map(|v| v.calc_value() as usize)
}

/// Blinc node ownership and styles with a native view for deferred CSS lengths.
pub struct LayoutTree {
    base: BaseTree,
    nodes: HashSet<LayoutNodeId>,
    pub(crate) expressions: HashMap<usize, Expression>,
    computed: HashMap<LayoutNodeId, Layout>,
    cache: HashMap<LayoutNodeId, Cache>,
    unrounded: HashMap<LayoutNodeId, Layout>,
    baselines: HashMap<LayoutNodeId, f32>,
    pub(crate) error: Option<&'static str>,
}
impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}
impl Deref for LayoutTree {
    type Target = BaseTree;
    fn deref(&self) -> &BaseTree {
        &self.base
    }
}
impl DerefMut for LayoutTree {
    fn deref_mut(&mut self) -> &mut BaseTree {
        // Every underlying mutation invalidates the native math view too.
        self.cache.clear();
        &mut self.base
    }
}
impl LayoutTree {
    pub fn new() -> Self {
        Self {
            base: BaseTree::new(),
            nodes: HashSet::new(),
            expressions: HashMap::new(),
            computed: HashMap::new(),
            cache: HashMap::new(),
            unrounded: HashMap::new(),
            baselines: HashMap::new(),
            error: None,
        }
    }
    pub fn create_node(&mut self, style: Style) -> LayoutNodeId {
        self.cache.clear();
        let n = self.base.create_node(style);
        self.nodes.insert(n);
        n
    }
    pub fn create_text_node(&mut self, style: Style, context: TextMeasureContext) -> LayoutNodeId {
        self.cache.clear();
        let n = self.base.create_text_node(style, context);
        self.nodes.insert(n);
        n
    }
    pub fn set_text_baseline(&mut self, id: LayoutNodeId, offset: f32) {
        self.cache.clear();
        self.baselines.insert(id, offset);
        self.base.set_text_baseline(id, offset);
    }
    pub fn refresh_text_baseline(&mut self, id: LayoutNodeId, offset: f32) {
        self.cache.clear();
        if let Some(b) = self.baselines.get_mut(&id) {
            *b = offset;
        }
        self.base.refresh_text_baseline(id, offset);
    }
    pub fn remove_node(&mut self, id: LayoutNodeId) {
        self.cache.clear();
        self.unrounded.remove(&id);
        self.nodes.remove(&id);
        self.computed.remove(&id);
        self.baselines.remove(&id);
        self.base.remove_node(id);
    }
    pub fn remove_subtree(&mut self, id: LayoutNodeId) {
        let mut nodes = vec![id];
        let mut at = 0;
        while at < nodes.len() {
            nodes.extend(self.base.children(nodes[at]));
            at += 1;
        }
        for n in nodes.into_iter().rev() {
            self.remove_node(n);
        }
    }
    pub fn clear_children(&mut self, id: LayoutNodeId) {
        for n in self.base.children(id) {
            self.remove_subtree(n);
        }
    }
    pub fn compute_layout(&mut self, root: LayoutNodeId, space: Size<AvailableSpace>) {
        self.error = None;
        // Removing/replacing declarations and nodes must release their expression storage.
        if !self.expressions.is_empty() {
            self.nodes.retain(|n| self.base.node_exists(*n));
            self.computed.retain(|n, _| self.nodes.contains(n));
            self.baselines.retain(|n, _| self.nodes.contains(n));
            let live: HashSet<_> = self
                .nodes
                .iter()
                .filter_map(|n| self.base.get_style(*n))
                .flat_map(|s| handles(&s).collect::<Vec<_>>())
                .collect();
            self.expressions.retain(|id, _| live.contains(id));
        }
        if self.expressions.is_empty() {
            self.computed.clear();
            self.cache.clear();
            self.unrounded.clear();
            self.base.compute_layout(root, space);
            return;
        }
        let mut view = View::new(
            &self.base,
            &self.expressions,
            &self.baselines,
            root,
            &mut self.cache,
            &self.unrounded,
        );
        compute_root_layout(&mut view, NodeId::from(0usize), space);
        round_layout(&mut view, NodeId::from(0usize));
        if view.failed.get() {
            self.error = Some("Invalid or non-finite CSS length during layout");
            return;
        }
        for node in view.nodes {
            self.computed.insert(node.id, node.final_layout);
            self.unrounded.insert(node.id, node.unrounded);
            self.cache.insert(node.id, node.cache);
        }
    }
    pub fn get_layout(&self, id: LayoutNodeId) -> Option<&Layout> {
        if !self.base.node_exists(id) {
            return None;
        }
        self.computed.get(&id).or_else(|| self.base.get_layout(id))
    }
    pub fn get_bounds(&self, id: LayoutNodeId, offset: (f32, f32)) -> Option<ElementBounds> {
        self.get_layout(id)
            .map(|l| ElementBounds::from_layout(l, offset))
    }
    pub fn get_absolute_bounds(&self, id: LayoutNodeId) -> Option<ElementBounds> {
        let mut offset = (0.0, 0.0);
        for parent in self.base.ancestors(id) {
            let l = self.get_layout(parent)?;
            offset.0 += l.location.x;
            offset.1 += l.location.y;
        }
        self.get_bounds(id, offset)
    }
    pub fn get_content_size(&self, id: LayoutNodeId) -> Option<(f32, f32)> {
        self.get_layout(id).map(|l| {
            (
                l.scrollable_overflow_rect.right,
                l.scrollable_overflow_rect.bottom,
            )
        })
    }
}

struct ViewNode {
    id: LayoutNodeId,
    style: Style,
    children: Vec<NodeId>,
    cache: Cache,
    unrounded: Layout,
    final_layout: Layout,
}
struct View<'a> {
    base: &'a BaseTree,
    expressions: &'a HashMap<usize, Expression>,
    baselines: &'a HashMap<LayoutNodeId, f32>,
    nodes: Vec<ViewNode>,
    failed: Cell<bool>,
}
impl<'a> View<'a> {
    fn new(
        base: &'a BaseTree,
        expressions: &'a HashMap<usize, Expression>,
        baselines: &'a HashMap<LayoutNodeId, f32>,
        root: LayoutNodeId,
        cache: &mut HashMap<LayoutNodeId, Cache>,
        unrounded: &HashMap<LayoutNodeId, Layout>,
    ) -> Self {
        let mut ids = vec![root];
        let mut i = 0;
        while i < ids.len() {
            ids.extend(base.children(ids[i]));
            i += 1;
        }
        let index: HashMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, NodeId::from(i)))
            .collect();
        let nodes = ids
            .into_iter()
            .map(|id| ViewNode {
                id,
                style: base.get_style(id).unwrap_or_default(),
                children: base.children(id).iter().map(|n| index[n]).collect(),
                cache: cache.remove(&id).unwrap_or_default(),
                unrounded: unrounded.get(&id).copied().unwrap_or_default(),
                final_layout: Layout::new(),
            })
            .collect();
        Self {
            base,
            expressions,
            baselines,
            nodes,
            failed: Cell::new(false),
        }
    }
    fn node(&self, n: NodeId) -> &ViewNode {
        &self.nodes[usize::from(n)]
    }
    fn node_mut(&mut self, n: NodeId) -> &mut ViewNode {
        &mut self.nodes[usize::from(n)]
    }
    fn compute(
        &mut self,
        n: NodeId,
        input: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        if input.run_mode == RunMode::PerformHiddenLayout {
            return compute_hidden_layout(self, n);
        }
        compute_cached_layout(self, n, input, |tree, n, input| {
            let node = tree.node(n);
            match (node.style.display, node.children.is_empty()) {
                (Display::None, _) => compute_hidden_layout(tree, n),
                (Display::Block, false) => compute_block_layout(tree, n, input, block),
                (Display::FlowRoot, false) => compute_block_layout(tree, n, input, None),
                (Display::Flex, false) => compute_flexbox_layout(tree, n, input),
                (Display::Grid, false) => compute_grid_layout(tree, n, input),
                (_, true) => tree.leaf(n, input),
            }
        })
    }
    fn leaf(&self, n: NodeId, input: LayoutInput) -> LayoutOutput {
        let node = self.node(n);
        let context = self.base.text_context(node.id);
        let calc = |handle, basis| self.resolve_calc_value(handle, basis);
        let mut output = compute_leaf_layout(input, &node.style, calc, |known, available| {
            if let (Some(w), Some(h)) = (known.width, known.height) {
                return Size {
                    width: w,
                    height: h,
                };
            }
            let Some(ctx) = context else {
                return Size::ZERO;
            };
            let mut options = options(ctx);
            if ctx.wrap {
                options.max_width = known.width.or(match available.width {
                    AvailableSpace::Definite(w) => Some(w),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                });
            }
            let m = measure_text_with_options(&ctx.content, ctx.font_size, &options);
            Size {
                width: known.width.unwrap_or(m.width),
                height: known.height.unwrap_or(m.height),
            }
        });
        let baseline = self.baselines.get(&node.id).copied().or_else(|| {
            context.map(|ctx| {
                let m = measure_text_with_options(&ctx.content, ctx.font_size, &options(ctx));
                (m.height / m.line_count.max(1) as f32 - (m.ascender - m.descender)) / 2.0
                    + m.ascender
            })
        });
        if let Some(b) = baseline {
            use taffy::util::MaybeResolve;
            let lead = node
                .style
                .border
                .top
                .maybe_resolve(input.parent_size.width, calc)
                .unwrap_or(0.0)
                + node
                    .style
                    .padding
                    .top
                    .maybe_resolve(input.parent_size.width, calc)
                    .unwrap_or(0.0);
            output.baselines = Baselines::from_first(Some(lead + b));
        }
        output
    }
}
fn options(ctx: &TextMeasureContext) -> TextLayoutOptions {
    let mut options = TextLayoutOptions::new();
    options.font_name = ctx.font_name.clone();
    options.generic_font = ctx.generic_font;
    options.font_weight = ctx.font_weight;
    options.italic = ctx.italic;
    options.line_height = ctx.line_height;
    options.letter_spacing = ctx.letter_spacing;
    options
}
impl TraversePartialTree for View<'_> {
    type ChildIter<'a>
        = std::iter::Copied<std::slice::Iter<'a, NodeId>>
    where
        Self: 'a;
    fn child_ids(&self, n: NodeId) -> Self::ChildIter<'_> {
        self.node(n).children.iter().copied()
    }
    fn child_count(&self, n: NodeId) -> usize {
        self.node(n).children.len()
    }
    fn get_child_id(&self, n: NodeId, i: usize) -> NodeId {
        self.node(n).children[i]
    }
}
impl TraverseTree for View<'_> {}
impl LayoutPartialTree for View<'_> {
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type CustomIdent = String;
    fn get_core_container_style(&self, n: NodeId) -> &Style {
        &self.node(n).style
    }
    fn set_unrounded_layout(&mut self, n: NodeId, l: &Layout) {
        self.node_mut(n).unrounded = *l;
    }
    fn resolve_calc_value(&self, handle: *const (), basis: f32) -> f32 {
        let resolved = self
            .expressions
            .get(&(handle as usize))
            .and_then(|e| e.try_resolve(basis as f64).ok())
            .map(|n| n as f32)
            .filter(|n| n.is_finite());
        match resolved {
            Some(n) => n,
            None => {
                self.failed.set(true);
                0.0
            }
        }
    }
    fn compute_child_layout(&mut self, n: NodeId, input: LayoutInput) -> LayoutOutput {
        self.compute(n, input, None)
    }
}
impl CacheTree for View<'_> {
    fn cache_get(&mut self, n: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        self.node_mut(n).cache.get(input)
    }
    fn cache_store(&mut self, n: NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.node_mut(n).cache.store(input, output);
    }
    fn cache_clear(&mut self, n: NodeId) {
        self.node_mut(n).cache.clear();
    }
}
impl LayoutBlockContainer for View<'_> {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;
    fn get_block_container_style(&self, n: NodeId) -> &Style {
        &self.node(n).style
    }
    fn get_block_child_style(&self, n: NodeId) -> &Style {
        &self.node(n).style
    }
    fn compute_block_child_layout(
        &mut self,
        n: NodeId,
        input: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        self.compute(n, input, block)
    }
}
impl LayoutFlexboxContainer for View<'_> {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;
    fn get_flexbox_container_style(&self, n: NodeId) -> &Style {
        &self.node(n).style
    }
    fn get_flexbox_child_style(&self, n: NodeId) -> &Style {
        &self.node(n).style
    }
}
impl LayoutGridContainer for View<'_> {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;
    fn get_grid_container_style(&self, n: NodeId) -> &Style {
        &self.node(n).style
    }
    fn get_grid_child_style(&self, n: NodeId) -> &Style {
        &self.node(n).style
    }
}
impl RoundTree for View<'_> {
    fn get_unrounded_layout(&self, n: NodeId) -> Layout {
        self.node(n).unrounded
    }
    fn set_final_layout(&mut self, n: NodeId, l: &Layout) {
        self.node_mut(n).final_layout = *l;
    }
}
