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
use taffy::style_helpers::{TaffyGridLine, line};
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
    // git-bug: blinc_abi 62aa0b0; Ashui 7977648. Only nonzero orders and
    // sorted parents need storage; authored children remain the document order.
    orders: HashMap<LayoutNodeId, i32>,
    authored: HashMap<LayoutNodeId, Vec<LayoutNodeId>>,
    pub(crate) expressions: HashMap<usize, Expression>,
    computed: HashMap<LayoutNodeId, Layout>,
    cache: HashMap<LayoutNodeId, Cache>,
    unrounded: HashMap<LayoutNodeId, Layout>,
    baselines: HashMap<LayoutNodeId, f32>,
    /// An opt-in layout projection; the base tree remains the authored tree.
    grid_rows: HashMap<LayoutNodeId, GridRows>,
    containment: HashMap<LayoutNodeId, crate::css::conditional::ContainerType>,
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
            orders: HashMap::new(),
            authored: HashMap::new(),
            expressions: HashMap::new(),
            computed: HashMap::new(),
            cache: HashMap::new(),
            unrounded: HashMap::new(),
            baselines: HashMap::new(),
            grid_rows: HashMap::new(),
            containment: HashMap::new(),
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
    /// Authored children, used by selectors, mutation and keyboard traversal.
    pub fn children(&self, parent: LayoutNodeId) -> Vec<LayoutNodeId> {
        self.authored
            .get(&parent)
            .cloned()
            .unwrap_or_else(|| self.base.children(parent))
    }
    /// Cached order-modified children, used by layout, painting and pointer hits.
    pub(crate) fn visual_children(&self, parent: LayoutNodeId) -> Vec<LayoutNodeId> {
        self.base.children(parent)
    }
    pub fn set_style(&mut self, node: LayoutNodeId, style: Style) {
        let display_changed = !self.orders.is_empty()
            && self
                .base
                .get_style(node)
                .is_some_and(|s| s.display != style.display);
        self.cache.clear();
        self.base.set_style(node, style);
        if display_changed {
            self.write_children(node, self.children(node));
        }
    }
    /// Reuse the owned-context algorithm for every adapter (git-bug: 62aa0b0).
    /// All changed siblings in a batch trigger a single stable sort of their parent.
    pub(crate) fn set_orders(
        &mut self,
        writes: impl IntoIterator<Item = (LayoutNodeId, i32)>,
    ) -> bool {
        let mut parents = Vec::new();
        let mut changed = false;
        for (id, order) in writes {
            if !self.base.node_exists(id) {
                continue;
            }
            let update = if order == 0 {
                self.orders.remove(&id).is_some()
            } else {
                self.orders.insert(id, order) != Some(order)
            };
            if update {
                changed = true;
                if let Some(&parent) = self.base.ancestors(id).first() {
                    parents.push(parent);
                }
            }
        }
        parents.sort_unstable();
        parents.dedup();
        for parent in parents {
            self.write_children(parent, self.children(parent));
        }
        changed
    }
    fn write_children(
        &mut self,
        parent: LayoutNodeId,
        list: Vec<LayoutNodeId>,
    ) -> Vec<LayoutNodeId> {
        self.cache.clear();
        let ordered = !self.orders.is_empty()
            && self
                .base
                .get_style(parent)
                .is_some_and(|s| matches!(s.display, Display::Flex | Display::Grid))
            && list.iter().any(|id| self.orders.contains_key(id));
        if ordered {
            let mut sorted = list.clone();
            sorted.sort_by_key(|id| self.orders.get(id).copied().unwrap_or(0));
            self.authored.insert(parent, list);
            self.base.replace_children(parent, sorted)
        } else {
            self.authored.remove(&parent);
            self.base.replace_children(parent, list)
        }
    }
    pub fn replace_children(
        &mut self,
        parent: LayoutNodeId,
        list: Vec<LayoutNodeId>,
    ) -> Vec<LayoutNodeId> {
        if !self.base.node_exists(parent) {
            return Vec::new();
        }
        // Moving a child also repairs its old parent's authored order. Most trees
        // have no orders, so the existing native mutation path needs no extra lists.
        if !self.orders.is_empty() {
            let previous: HashSet<_> = list
                .iter()
                .filter_map(|&id| self.base.ancestors(id).first().copied())
                .filter(|&old| old != parent)
                .collect();
            if !previous.is_empty() {
                let moved: HashSet<_> = list.iter().copied().collect();
                for old in previous {
                    let mut siblings = self.children(old);
                    siblings.retain(|id| !moved.contains(id));
                    self.write_children(old, siblings);
                }
            }
        }
        let authored = self.authored.get(&parent).cloned();
        let old = self.write_children(parent, list);
        authored.unwrap_or(old)
    }
    pub fn add_child(&mut self, parent: LayoutNodeId, child: LayoutNodeId) {
        self.cache.clear();
        if self.orders.is_empty() {
            self.base.add_child(parent, child);
            return;
        }
        if !self.base.node_exists(parent) || !self.base.node_exists(child) {
            return;
        }
        if !self.authored.contains_key(&parent)
            && !self.orders.contains_key(&child)
            && self.base.ancestors(child).is_empty()
        {
            self.base.add_child(parent, child);
        } else {
            let mut list = self.children(parent);
            list.retain(|&id| id != child);
            list.push(child);
            self.replace_children(parent, list);
        }
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
        self.grid_rows.remove(&id);
        self.containment.remove(&id);
        self.grid_rows.retain(|_, rows| !rows.contains(&id));
        self.orders.remove(&id);
        self.authored.remove(&id);
        let parent = self.base.ancestors(id).first().copied();
        self.base.remove_node(id);
        if let Some(parent) = parent
            && let Some(list) = self.authored.get_mut(&parent)
        {
            list.retain(|&child| child != id);
            let list = list.clone();
            self.write_children(parent, list);
        }
    }
    /// Lay these direct rows' children out together. Empty rows restore ordinary layout.
    /// Row and cell identities, styles and logical ancestry stay unchanged.
    pub fn set_grid_rows(
        &mut self,
        group: LayoutNodeId,
        rows: &[LayoutNodeId],
    ) -> Result<(), &'static str> {
        if !self.base.node_exists(group) {
            return Err("Grid group is removed");
        }
        let children = self.base.children(group);
        let direct: HashSet<_> = children.iter().copied().collect();
        let unique: HashSet<_> = rows.iter().copied().collect();
        if children.len() >= i16::MAX as usize
            || unique.len() != rows.len()
            || rows.iter().any(|row| !direct.contains(row))
        {
            return Err("Grid rows must be unique direct children of their group");
        }
        // The grid's row order is its authored order, never the supplied list's order.
        let ordered: Vec<_> = children
            .into_iter()
            .filter(|n| unique.contains(n))
            .collect();
        if ordered.is_empty() {
            self.grid_rows.remove(&group);
        } else if self
            .grid_rows
            .get(&group)
            .is_some_and(|r| r.rows == ordered)
        {
            return Ok(());
        } else {
            self.grid_rows.insert(
                group,
                GridRows {
                    rows: ordered,
                    members: unique,
                },
            );
        }
        self.cache.clear();
        self.computed.clear();
        self.unrounded.clear();
        let style = self.base.get_style(group).ok_or("Grid group is removed")?;
        self.base.set_style(group, style);
        Ok(())
    }
    #[cfg(feature = "scene")]
    pub(crate) fn grid_rows(&self, group: LayoutNodeId) -> Option<&GridRows> {
        self.grid_rows.get(&group)
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
        // A detached or reparented row cannot leave a stale projection behind.
        self.grid_rows.retain(|group, rows| {
            let direct: HashSet<_> = self.base.children(*group).into_iter().collect();
            self.base.node_exists(*group)
                && direct.len() < i16::MAX as usize
                && rows.iter().all(|row| direct.contains(row))
        });
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
        if self.expressions.is_empty() && self.grid_rows.is_empty() && self.containment.is_empty() {
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
            &self.containment,
            root,
            &mut self.cache,
            &self.unrounded,
        );
        let projected = if self.grid_rows.is_empty() {
            Vec::new()
        } else {
            view.project_rows(&self.grid_rows)
        };
        compute_root_layout(&mut view, NodeId::from(0usize), space);
        round_layout(&mut view, NodeId::from(0usize));
        if view.failed.get() {
            self.error = Some("Invalid or non-finite CSS length during layout");
            return;
        }
        for (cell, row) in projected {
            let origin = view.node(row).final_layout.location;
            let cell = view.node_mut(cell);
            cell.final_layout.location.x -= origin.x;
            cell.final_layout.location.y -= origin.y;
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
    pub fn set_containment(
        &mut self,
        id: LayoutNodeId,
        kind: crate::css::conditional::ContainerType,
    ) -> bool {
        use crate::css::conditional::ContainerType;
        if self.containment.get(&id).copied().unwrap_or_default() == kind {
            return false;
        }
        if kind == ContainerType::Normal {
            self.containment.remove(&id);
        } else {
            self.containment.insert(id, kind);
        }
        self.cache.clear();
        true
    }
    /// Query sizes use the unrounded principal content box, in CSS pixels.
    pub fn container_size(&self, id: LayoutNodeId) -> Option<(f64, f64)> {
        for ancestor in std::iter::once(id).chain(self.base.ancestors(id)) {
            if self.base.get_style(ancestor)?.display == Display::None {
                return None;
            }
        }
        let layout = self
            .unrounded
            .get(&id)
            .or_else(|| self.base.get_layout(id))?;
        Some((
            (layout.size.width
                - layout.padding.left
                - layout.padding.right
                - layout.border.left
                - layout.border.right
                - layout.scrollbar_size.width)
                .max(0.0) as f64,
            (layout.size.height
                - layout.padding.top
                - layout.padding.bottom
                - layout.border.top
                - layout.border.bottom
                - layout.scrollbar_size.height)
                .max(0.0) as f64,
        ))
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

pub(crate) struct GridRows {
    rows: Vec<LayoutNodeId>,
    members: HashSet<LayoutNodeId>,
}
impl Deref for GridRows {
    type Target = [LayoutNodeId];
    fn deref(&self) -> &[LayoutNodeId] {
        &self.rows
    }
}
impl GridRows {
    pub(crate) fn contains(&self, node: &LayoutNodeId) -> bool {
        self.members.contains(node)
    }
}

struct ViewNode {
    id: LayoutNodeId,
    style: Style,
    children: Vec<NodeId>,
    cache: Cache,
    unrounded: Layout,
    final_layout: Layout,
    containment: crate::css::conditional::ContainerType,
}
struct View<'a> {
    base: &'a BaseTree,
    expressions: &'a HashMap<usize, Expression>,
    baselines: &'a HashMap<LayoutNodeId, f32>,
    nodes: Vec<ViewNode>,
    failed: Cell<bool>,
}
impl<'a> View<'a> {
    /// Flatten only the layout graph, with rows as full-width track-sizing boxes.
    /// Cells are rebased under their authored row after rounding.
    fn project_rows(&mut self, groups: &HashMap<LayoutNodeId, GridRows>) -> Vec<(NodeId, NodeId)> {
        let index: HashMap<_, _> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, NodeId::from(i)))
            .collect();
        let mut projected = Vec::new();
        for (group, rows) in groups {
            let Some(&group) = index.get(group) else {
                continue;
            };
            if self.node(group).style.display == Display::None {
                continue;
            }
            let rows: Vec<_> = rows
                .iter()
                .filter_map(|id| index.get(id).copied())
                .collect();
            let Some(&first) = rows.first() else { continue };
            let ordinal: HashMap<_, _> = rows.iter().enumerate().map(|(i, n)| (*n, i)).collect();
            let columns = self.node(first).style.grid_template_columns.clone();
            let gap = self.node(first).style.gap.width;
            let children = self.node(group).children.clone();
            let mut slots = HashMap::new();
            let mut slot = 1i16;
            for &child in &children {
                if self.node(child).style.display != Display::None {
                    slots.insert(child, slot);
                    slot += 1;
                }
            }
            let mut flat = Vec::new();
            for child in children {
                let Some(&ordinal) = ordinal.get(&child) else {
                    let style = &mut self.node_mut(child).style;
                    style.grid_column = crate::grid::line_pair("1 / -1").unwrap();
                    if let Some(&slot) = slots.get(&child) {
                        style.grid_row = line(slot);
                    }
                    flat.push(child);
                    continue;
                };
                let Some(&slot) = slots.get(&child) else {
                    // Hidden rows keep their subtree so Taffy clears its hidden layout.
                    flat.push(child);
                    continue;
                };
                let cells = std::mem::take(&mut self.node_mut(child).children);
                let row = self.node_mut(child);
                row.style.grid_column = crate::grid::line_pair("1 / -1").unwrap();
                row.style.grid_row = line(slot);
                // A row is the track's own box; its children are measured by the shared grid.
                row.style.align_self = Some(AlignItems::STRETCH);
                row.style.justify_self = Some(AlignItems::STRETCH);
                flat.push(child);
                for cell in cells {
                    let span = match self.node(cell).style.grid_row.end {
                        GridPlacement::Span(n) => usize::from(n),
                        _ => 1,
                    };
                    let end = rows
                        .iter()
                        .skip(ordinal)
                        .take(span)
                        .filter_map(|r| slots.get(r))
                        .next_back()
                        .copied()
                        .unwrap_or(slot);
                    self.node_mut(cell).style.grid_row = Line {
                        start: GridPlacement::from_line_index(slot),
                        end: GridPlacement::from_line_index(end + 1),
                    };
                    flat.push(cell);
                    projected.push((cell, child));
                }
            }
            let node = self.node_mut(group);
            node.children = flat;
            node.style.display = Display::Grid;
            node.style.grid_template_columns = columns;
            node.style.gap.width = gap;
        }
        projected
    }
    fn new(
        base: &'a BaseTree,
        expressions: &'a HashMap<usize, Expression>,
        baselines: &'a HashMap<LayoutNodeId, f32>,
        containment: &HashMap<LayoutNodeId, crate::css::conditional::ContainerType>,
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
                containment: containment.get(&id).copied().unwrap_or_default(),
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
        compute_cached_layout(self, n, input, |tree, n, mut input| {
            let node = tree.node(n);
            use crate::css::conditional::ContainerType;
            let contained =
                node.containment != ContainerType::Normal && node.style.display != Display::None;
            if contained {
                let empty = compute_leaf_layout(
                    input,
                    &node.style,
                    |handle, basis| tree.resolve_calc_value(handle, basis),
                    |_, _| Size::ZERO,
                );
                input.known_dimensions.width.get_or_insert(empty.size.width);
                if node.containment == ContainerType::Size {
                    input
                        .known_dimensions
                        .height
                        .get_or_insert(empty.size.height);
                }
            }
            let node = tree.node(n);
            match (node.style.display, node.children.is_empty()) {
                (Display::None, _) => compute_hidden_layout(tree, n),
                (Display::Block, false) => {
                    compute_block_layout(tree, n, input, if contained { None } else { block })
                }
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

#[cfg(test)]
mod order_tests {
    use super::*;
    fn parent(tree: &mut LayoutTree, display: Display) -> LayoutNodeId {
        tree.create_node(Style {
            display,
            ..Style::DEFAULT
        })
    }
    #[test]
    fn shared_order_keeps_authored_children_and_stable_integer_ties() {
        let mut tree = LayoutTree::new();
        let root = parent(&mut tree, Display::Flex);
        let a = tree.create_node(Style::DEFAULT);
        let b = tree.create_node(Style::DEFAULT);
        let c = tree.create_node(Style::DEFAULT);
        tree.replace_children(root, vec![a, b, c]);
        assert!(tree.set_orders([(a, i32::MAX), (b, i32::MIN), (c, i32::MAX - 1)]));
        assert_eq!(tree.children(root), vec![a, b, c]);
        assert_eq!(tree.visual_children(root), vec![b, c, a]);
        tree.set_orders([(a, -1), (c, -1)]);
        assert_eq!(tree.visual_children(root), vec![b, a, c]);
        assert!(!tree.set_orders([(a, -1)]));
        tree.set_orders([(a, 0), (b, 0), (c, 0)]);
        assert_eq!(tree.visual_children(root), vec![a, b, c]);
        assert!(tree.orders.is_empty() && tree.authored.is_empty());
    }
    #[test]
    fn order_only_affects_flex_and_grid_and_survives_display_changes() {
        let mut tree = LayoutTree::new();
        let root = parent(&mut tree, Display::Block);
        let a = tree.create_node(Style::DEFAULT);
        let b = tree.create_node(Style::DEFAULT);
        tree.replace_children(root, vec![a, b]);
        tree.set_orders([(a, 1)]);
        assert_eq!(tree.visual_children(root), vec![a, b]);
        for display in [Display::Flex, Display::Grid, Display::Block, Display::Flex] {
            tree.set_style(
                root,
                Style {
                    display,
                    ..Style::DEFAULT
                },
            );
            assert_eq!(
                tree.visual_children(root),
                if display == Display::Block {
                    vec![a, b]
                } else {
                    vec![b, a]
                }
            );
            assert_eq!(tree.children(root), vec![a, b]);
        }
    }
    #[test]
    fn order_tracks_appends_replacements_moves_detachment_and_deletion() {
        let mut tree = LayoutTree::new();
        let p = parent(&mut tree, Display::Flex);
        let q = parent(&mut tree, Display::Grid);
        let a = tree.create_node(Style::DEFAULT);
        let b = tree.create_node(Style::DEFAULT);
        let c = tree.create_node(Style::DEFAULT);
        tree.set_orders([(a, 2), (c, -1)]);
        tree.add_child(p, a);
        tree.add_child(p, b);
        tree.add_child(p, c);
        assert_eq!(tree.visual_children(p), vec![c, b, a]);
        assert_eq!(tree.replace_children(p, vec![b, c, a]), vec![a, b, c]);
        assert_eq!(tree.children(p), vec![b, c, a]);
        tree.add_child(q, c);
        assert_eq!(tree.children(p), vec![b, a]);
        assert_eq!(tree.visual_children(p), vec![b, a]);
        tree.replace_children(q, vec![a, c]);
        assert_eq!(tree.children(p), vec![b]);
        assert!(!tree.authored.contains_key(&p));
        assert_eq!(tree.visual_children(q), vec![c, a]);
        tree.remove_node(c);
        assert_eq!(tree.children(q), vec![a]);
        tree.replace_children(q, vec![]);
        assert_eq!(tree.children(q), vec![]);
        tree.add_child(p, a);
        assert_eq!(tree.visual_children(p), vec![b, a]);
        tree.remove_subtree(p);
        tree.remove_node(q);
        assert!(tree.orders.is_empty() && tree.authored.is_empty());
    }
}
