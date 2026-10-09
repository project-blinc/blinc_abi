//! Owned layout contexts for language adapters. No host GC or process-global tree.
use blinc_layout::element::RenderProps;
use blinc_layout::tree::{LayoutNodeId, LayoutTree};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
pub use taffy::prelude::{AvailableSpace, Size, Style};

static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);

/// A generation-checked node, valid only in its originating context.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Node {
    pub(crate) context: u64,
    pub(crate) id: LayoutNodeId,
}

impl Node {
    /// Generation-bearing identity, unique within this context.
    pub fn raw(self) -> u64 {
        self.id.to_raw()
    }
}

/// Each adapter owns a context and serializes access to it on its host thread.
/// Styles are Taffy's complete styles; SDKs choose their authored style surface.
pub struct LayoutContext {
    pub(crate) id: u64,
    pub(crate) tree: Option<crate::tree::Tree>,
    pub(crate) revision: u64,
    pub(crate) parents: HashMap<LayoutNodeId, LayoutNodeId>,
    pub(crate) bounds: HashMap<LayoutNodeId, [f32; 4]>,
    /// CSS `order` of nodes whose order is not 0. Taffy has no `order`, so a
    /// parent with such a child is laid out from its children stably sorted.
    orders: HashMap<LayoutNodeId, i32>,
    /// Children as authored, for parents laid out from a sorted list.
    authored: HashMap<LayoutNodeId, Vec<LayoutNodeId>>,
}

/// A value written through the property router (`crate::layout_props` ids).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropValue<'a> {
    Number(f32),
    Enum(i32),
    Text(Option<&'a str>),
    /// Back to a new node's value.
    Unset,
}

/// Owned-context ids beyond the shared router: border widths that also take
/// layout space, as CSS's box model does, and CSS `order`.
pub const BORDER_WIDTH: i32 = 2;
pub const BORDER_TOP_WIDTH: i32 = 60;
pub const ORDER: i32 = 118;

type Result<T> = std::result::Result<T, &'static str>;

/// A border width, or `None` to unset it: paint in `props` and layout space
/// in `style`, top, right, bottom, left for the side ids.
fn set_border(style: &mut Style, props: &mut RenderProps, raw: i32, width: Option<f32>) {
    use taffy::prelude::LengthPercentage;
    let px = LengthPercentage::length(width.unwrap_or(0.0));
    if raw == BORDER_WIDTH {
        style.border = taffy::Rect {
            left: px,
            right: px,
            top: px,
            bottom: px,
        };
        props.border_width = width.unwrap_or(RenderProps::default().border_width);
        return;
    }
    let sides = &mut props.border_sides;
    let (layout, paint) = match raw - BORDER_TOP_WIDTH {
        0 => (&mut style.border.top, &mut sides.top),
        1 => (&mut style.border.right, &mut sides.right),
        2 => (&mut style.border.bottom, &mut sides.bottom),
        _ => (&mut style.border.left, &mut sides.left),
    };
    *layout = px;
    crate::layout_props::border_side(paint).width = width.unwrap_or(-1.0);
}

impl Default for LayoutContext {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutContext {
    pub fn new() -> Self {
        Self {
            id: NEXT_CONTEXT.fetch_add(1, Ordering::Relaxed),
            tree: Some(crate::tree::Tree::new()),
            revision: 0,
            parents: HashMap::new(),
            bounds: HashMap::new(),
            orders: HashMap::new(),
            authored: HashMap::new(),
        }
    }

    fn tree(&self) -> Result<&LayoutTree> {
        self.tree
            .as_ref()
            .map(|tree| &tree.layout)
            .ok_or("Layout context is disposed")
    }
    pub(crate) fn check(&self, node: Node) -> Result<LayoutNodeId> {
        let tree = self.tree()?;
        if node.context != self.id {
            return Err("Node belongs to another layout context");
        }
        if !tree.node_exists(node.id) {
            return Err("Layout node is removed");
        }
        Ok(node.id)
    }

    /// The live node with generation-bearing identity `raw` (`Node::raw`) in this context.
    pub fn node(&self, raw: u64) -> Result<Node> {
        let node = Node {
            context: self.id,
            id: LayoutNodeId::from_raw(raw),
        };
        self.check(node)?;
        Ok(node)
    }

    pub fn create_node(&mut self, style: Style) -> Result<Node> {
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        self.revision += 1;
        Ok(Node {
            context: self.id,
            id: tree.create_node(style),
        })
    }
    pub fn style(&self, node: Node) -> Result<Style> {
        self.tree()?
            .get_style(self.check(node)?)
            .ok_or("Layout node is removed")
    }
    pub fn set_style(&mut self, node: Node, style: Style) -> Result<()> {
        let id = self.check(node)?;
        self.tree
            .as_mut()
            .ok_or("Layout context is disposed")?
            .set_style(id, style);
        self.bounds.clear();
        self.revision += 1;
        Ok(())
    }

    /// Apply property-router writes in order. Every node's style and paint
    /// are staged first, so an unknown id or a value of the wrong type
    /// changes nothing. Each node's style is then set once.
    pub fn apply(&mut self, writes: &[(Node, i32, PropValue<'_>)]) -> Result<()> {
        struct Staged {
            id: LayoutNodeId,
            style: Style,
            props: Option<RenderProps>,
            order: Option<i32>,
        }
        let mut staged: Vec<Staged> = Vec::new();
        let mut index: HashMap<LayoutNodeId, usize> = HashMap::new();
        {
            let tree = self.tree.as_ref().ok_or("Layout context is disposed")?;
            for &(node, raw, value) in writes {
                let id = self.check(node)?;
                let slot = *index.entry(id).or_insert_with(|| {
                    staged.push(Staged {
                        id,
                        style: tree.layout.get_style(id).unwrap_or_default(),
                        props: None,
                        order: None,
                    });
                    staged.len() - 1
                });
                let entry = &mut staged[slot];
                let border =
                    raw == BORDER_WIDTH || (BORDER_TOP_WIDTH..BORDER_TOP_WIDTH + 4).contains(&raw);
                let applied = match value {
                    PropValue::Number(v) if v.is_infinite() => false,
                    _ if raw == ORDER => match value {
                        PropValue::Enum(v) => {
                            entry.order = Some(v);
                            true
                        }
                        PropValue::Unset => {
                            entry.order = Some(0);
                            true
                        }
                        _ => false,
                    },
                    _ if border => {
                        let props = entry.props.get_or_insert_with(|| {
                            tree.props.get(&id).cloned().unwrap_or_default()
                        });
                        match value {
                            PropValue::Number(v) if v >= 0.0 => {
                                set_border(&mut entry.style, props, raw, Some(v));
                                true
                            }
                            PropValue::Unset => {
                                set_border(&mut entry.style, props, raw, None);
                                true
                            }
                            _ => false,
                        }
                    }
                    PropValue::Number(v) => crate::layout_props::set_f32(&mut entry.style, raw, v),
                    PropValue::Enum(v) => crate::layout_props::set_i32(&mut entry.style, raw, v),
                    PropValue::Text(v) => crate::layout_props::set_string(&mut entry.style, raw, v),
                    PropValue::Unset => crate::layout_props::unset(&mut entry.style, raw),
                };
                if !applied {
                    return Err("Unknown layout property, or a value of the wrong type");
                }
            }
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        let mut reordered = Vec::new();
        for entry in staged {
            tree.layout.set_style(entry.id, entry.style);
            if let Some(props) = entry.props {
                tree.props.insert(entry.id, props);
            }
            if let Some(order) = entry.order {
                let changed = if order == 0 {
                    self.orders.remove(&entry.id).is_some()
                } else {
                    self.orders.insert(entry.id, order) != Some(order)
                };
                if changed && let Some(&parent) = self.parents.get(&entry.id) {
                    reordered.push(parent);
                }
            }
        }
        reordered.sort_unstable();
        reordered.dedup();
        for parent in reordered {
            let list = self.child_list(parent);
            self.write_children(parent, list);
        }
        self.bounds.clear();
        self.revision += 1;
        Ok(())
    }

    /// Children in authored order, before any sorting by `order`.
    fn child_list(&self, parent: LayoutNodeId) -> Vec<LayoutNodeId> {
        match self.authored.get(&parent) {
            Some(list) => list.clone(),
            None => self
                .tree
                .as_ref()
                .map(|tree| tree.layout.children(parent))
                .unwrap_or_default(),
        }
    }

    /// Lay `parent` out from `list`, stably sorted by `order` when any child has one.
    fn write_children(&mut self, parent: LayoutNodeId, list: Vec<LayoutNodeId>) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };
        if list.iter().any(|id| self.orders.contains_key(id)) {
            let mut sorted = list.clone();
            sorted.sort_by_key(|id| self.orders.get(id).copied().unwrap_or(0));
            tree.layout.replace_children(parent, sorted);
            self.authored.insert(parent, list);
        } else {
            self.authored.remove(&parent);
            tree.layout.replace_children(parent, list);
        }
    }

    /// The node's children, in the order they were placed.
    pub fn children(&self, node: Node) -> Result<Vec<Node>> {
        let id = self.check(node)?;
        Ok(self
            .child_list(id)
            .into_iter()
            .map(|id| Node {
                context: self.id,
                id,
            })
            .collect())
    }

    /// The node's parent, if it has one.
    pub fn parent(&self, node: Node) -> Result<Option<Node>> {
        let id = self.check(node)?;
        Ok(self.parents.get(&id).map(|&id| Node {
            context: self.id,
            id,
        }))
    }

    /// Place `child` under `parent` before `before`, or last when `before`
    /// is `None`, first detaching it from wherever it is. Placing a node
    /// before itself changes nothing.
    pub fn insert_before(&mut self, parent: Node, child: Node, before: Option<Node>) -> Result<()> {
        let parent = self.check(parent)?;
        let child = self.check(child)?;
        let before = before.map(|node| self.check(node)).transpose()?;
        if before == Some(child) {
            return Ok(());
        }
        if before.is_some_and(|id| self.parents.get(&id) != Some(&parent)) {
            return Err("Reference node is not a child of the parent");
        }
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            if id == child {
                return Err("Layout edit would create a cycle");
            }
            ancestor = self.parents.get(&id).copied();
        }
        let old = self.parents.get(&child).copied();
        if old.is_none()
            && before.is_none()
            && !self.authored.contains_key(&parent)
            && !self.orders.contains_key(&child)
        {
            // Appending a free node needs no list rebuild.
            self.tree
                .as_mut()
                .ok_or("Layout context is disposed")?
                .layout
                .add_child(parent, child);
        } else {
            if let Some(old) = old.filter(|&old| old != parent) {
                let mut siblings = self.child_list(old);
                siblings.retain(|&id| id != child);
                self.write_children(old, siblings);
            }
            let mut list = self.child_list(parent);
            list.retain(|&id| id != child);
            let at = before
                .and_then(|b| list.iter().position(|&id| id == b))
                .unwrap_or(list.len());
            list.insert(at, child);
            self.write_children(parent, list);
        }
        self.parents.insert(child, parent);
        self.bounds.clear();
        self.revision += 1;
        Ok(())
    }

    /// Take `node` out of its parent without removing it; it can be placed again.
    pub fn detach(&mut self, node: Node) -> Result<()> {
        let id = self.check(node)?;
        if let Some(parent) = self.parents.remove(&id) {
            let mut list = self.child_list(parent);
            list.retain(|&child| child != id);
            self.write_children(parent, list);
            self.bounds.clear();
            self.revision += 1;
        }
        Ok(())
    }

    /// Replace/reorder children, detaching moved children from their old parents.
    /// Validate the entire edit before touching the tree.
    pub fn set_children(&mut self, parent: Node, children: &[Node]) -> Result<()> {
        let parent = self.check(parent)?;
        let mut seen = HashSet::with_capacity(children.len());
        let mut ids = Vec::with_capacity(children.len());
        let mut ancestors = HashSet::new();
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            ancestors.insert(id);
            ancestor = self.parents.get(&id).copied();
        }
        for &child in children {
            let child = self.check(child)?;
            if !seen.insert(child) {
                return Err("Duplicate child in layout tree");
            }
            if ancestors.contains(&child) {
                return Err("Layout edit would create a cycle");
            }
            ids.push(child);
        }
        // Rebuild each previous parent's child list only once for a bulk move.
        let old_parents: HashSet<_> = ids
            .iter()
            .filter_map(|id| self.parents.get(id).copied())
            .filter(|&id| id != parent)
            .collect();
        for old_parent in old_parents {
            let mut siblings = self.child_list(old_parent);
            siblings.retain(|id| !seen.contains(id));
            self.write_children(old_parent, siblings);
        }
        for old in self.child_list(parent) {
            self.parents.remove(&old);
        }
        for &child in &ids {
            self.parents.insert(child, parent);
        }
        self.write_children(parent, ids);
        self.bounds.clear();
        self.revision += 1;
        Ok(())
    }

    pub fn remove(&mut self, node: Node) -> Result<()> {
        let id = self.check(node)?;
        if let Some(parent) = self.parents.get(&id).copied()
            && let Some(list) = self.authored.get_mut(&parent)
        {
            list.retain(|&child| child != id);
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        let mut nodes = vec![id];
        let mut index = 0;
        while index < nodes.len() {
            nodes.extend(tree.children(nodes[index]));
            index += 1;
        }
        // Iterative removal also handles deeply nested authored trees.
        for id in nodes.into_iter().rev() {
            self.parents.remove(&id);
            self.orders.remove(&id);
            self.authored.remove(&id);
            tree.remove_node(id);
            tree.forget(id);
        }
        self.bounds.clear();
        self.revision += 1;
        Ok(())
    }

    pub fn compute(&mut self, root: Node, width: f32, height: f32) -> Result<()> {
        let id = self.check(root)?;
        if !width.is_finite() || !height.is_finite() || width < 0.0 || height < 0.0 {
            return Err("Layout size must be finite and non-negative");
        }
        if self.parents.contains_key(&id) {
            return Err("Layout root must have no parent");
        }
        self.tree
            .as_mut()
            .ok_or("Layout context is disposed")?
            .compute_layout(
                id,
                Size {
                    width: AvailableSpace::Definite(width),
                    height: AvailableSpace::Definite(height),
                },
            );
        self.revision += 1;
        // Resolve absolute coordinates once, not once per queried node and ancestor.
        let tree = self.tree.as_ref().ok_or("Layout context is disposed")?;
        let mut pending = vec![(id, (0.0, 0.0))];
        while let Some((node, parent_offset)) = pending.pop() {
            let bounds = tree
                .get_bounds(node, parent_offset)
                .ok_or("Layout node is removed")?;
            self.bounds
                .insert(node, [bounds.x, bounds.y, bounds.width, bounds.height]);
            pending.extend(
                tree.children(node)
                    .into_iter()
                    .map(|child| (child, (bounds.x, bounds.y))),
            );
        }
        Ok(())
    }

    /// Write absolute x, y, width, height for each requested node into reusable storage.
    /// All handles and capacity are checked before any output is changed.
    pub fn read_bounds(&self, nodes: &[Node], output: &mut [f32]) -> Result<()> {
        self.tree()?;
        if output.len() / 4 < nodes.len() {
            return Err("Layout output is too small");
        }
        for &node in nodes {
            let id = self.check(node)?;
            if !self.bounds.contains_key(&id) {
                return Err("Compute layout before reading bounds");
            }
        }
        for (&node, out) in nodes.iter().zip(output.as_chunks_mut::<4>().0.iter_mut()) {
            out.copy_from_slice(&self.bounds[&node.id]);
        }
        Ok(())
    }

    /// How far what is laid out inside `node` reaches, right and down from
    /// its top-left: past its size when content overflows, which bounds scrolling.
    pub fn content_size(&self, node: Node) -> Result<[f32; 2]> {
        let id = self.check(node)?;
        if !self.bounds.contains_key(&id) {
            return Err("Compute layout before reading bounds");
        }
        let (width, height) = self
            .tree()?
            .get_content_size(id)
            .ok_or("Layout node is removed")?;
        Ok([width, height])
    }

    pub fn len(&self) -> Result<usize> {
        Ok(self.tree()?.len())
    }
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.tree()?.is_empty())
    }
    pub fn dispose(&mut self) {
        self.tree.take();
        self.parents = HashMap::new();
        self.bounds = HashMap::new();
        self.orders = HashMap::new();
        self.authored = HashMap::new();
    }
    pub fn is_disposed(&self) -> bool {
        self.tree.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::prelude::*;
    fn box_style(w: f32, h: f32) -> Style {
        Style {
            size: Size {
                width: Dimension::length(w),
                height: Dimension::length(h),
            },
            flex_shrink: 0.0,
            ..Default::default()
        }
    }
    #[test]
    fn layout_reorder_reparent_and_invalidation() {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(box_style(200.0, 80.0)).unwrap();
        let a = ctx.create_node(box_style(40.0, 30.0)).unwrap();
        let b = ctx.create_node(box_style(60.0, 20.0)).unwrap();
        ctx.set_children(root, &[a, b]).unwrap();
        ctx.compute(root, 200.0, 80.0).unwrap();
        let mut out = [0.0; 8];
        ctx.read_bounds(&[a, b], &mut out).unwrap();
        assert_eq!(out, [0.0, 0.0, 40.0, 30.0, 40.0, 0.0, 60.0, 20.0]);
        ctx.set_children(root, &[b, a]).unwrap();
        assert!(ctx.read_bounds(&[a], &mut out).is_err());
        ctx.compute(root, 200.0, 80.0).unwrap();
        ctx.read_bounds(&[a, b], &mut out).unwrap();
        assert_eq!(out[0], 60.0);
        ctx.set_children(b, &[a]).unwrap();
        ctx.compute(root, 200.0, 80.0).unwrap();
        ctx.read_bounds(&[a], &mut out).unwrap();
        assert_eq!(out[0], 0.0);
        ctx.remove(b).unwrap();
        assert_eq!(ctx.len().unwrap(), 1);
        assert!(ctx.style(a).is_err());
    }
    #[test]
    fn rejects_invalid_edits_and_handles_without_partial_writes() {
        let mut ctx = LayoutContext::new();
        let mut other = LayoutContext::new();
        let root = ctx.create_node(Style::default()).unwrap();
        let child = ctx.create_node(Style::default()).unwrap();
        let foreign = other.create_node(Style::default()).unwrap();
        ctx.set_children(root, &[child]).unwrap();
        assert!(ctx.set_children(child, &[root]).is_err());
        assert!(ctx.set_children(root, &[child, child]).is_err());
        assert!(ctx.set_children(root, &[foreign]).is_err());
        assert!(ctx.compute(root, f32::NAN, 10.0).is_err());
        ctx.compute(root, 100.0, 100.0).unwrap();
        let mut out = [9.0; 8];
        assert!(ctx.read_bounds(&[root, foreign], &mut out).is_err());
        assert_eq!(out, [9.0; 8]);
        ctx.remove(child).unwrap();
        let replacement = ctx.create_node(Style::default()).unwrap();
        assert_ne!(child, replacement);
        assert!(ctx.style(child).is_err());
        assert_eq!(ctx.node(root.raw()), Ok(root));
        assert!(ctx.node(child.raw()).is_err());
        ctx.dispose();
        ctx.dispose();
        assert!(ctx.create_node(Style::default()).is_err());
    }
    #[test]
    fn disposed_context_rejects_access_without_changing_output() {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(box_style(100.0, 100.0)).unwrap();
        ctx.compute(root, 100.0, 100.0).unwrap();
        ctx.dispose();
        ctx.dispose();
        let mut out = [9.0; 4];
        assert_eq!(
            ctx.create_node(Style::default()).unwrap_err(),
            "Layout context is disposed"
        );
        assert_eq!(ctx.style(root).unwrap_err(), "Layout context is disposed");
        assert_eq!(
            ctx.set_style(root, Style::default()),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.set_children(root, &[]),
            Err("Layout context is disposed")
        );
        assert_eq!(ctx.remove(root), Err("Layout context is disposed"));
        assert_eq!(
            ctx.compute(root, 100.0, 100.0),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.read_bounds(&[root], &mut out),
            Err("Layout context is disposed")
        );
        assert_eq!(out, [9.0; 4]);
        assert_eq!(ctx.len(), Err("Layout context is disposed"));
        assert_eq!(ctx.is_empty(), Err("Layout context is disposed"));
    }
    fn x_positions(ctx: &mut LayoutContext, root: Node, nodes: &[Node]) -> Vec<f32> {
        ctx.compute(root, 200.0, 80.0).unwrap();
        let mut out = vec![0.0; nodes.len() * 4];
        ctx.read_bounds(nodes, &mut out).unwrap();
        out.chunks(4).map(|b| b[0]).collect()
    }
    #[test]
    fn incremental_child_operations() {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(box_style(200.0, 80.0)).unwrap();
        let other = ctx.create_node(box_style(200.0, 80.0)).unwrap();
        let [a, b, c] = [10.0, 20.0, 30.0].map(|w| ctx.create_node(box_style(w, 10.0)).unwrap());
        ctx.insert_before(root, a, None).unwrap();
        ctx.insert_before(root, c, None).unwrap();
        ctx.insert_before(root, b, Some(c)).unwrap();
        assert_eq!(x_positions(&mut ctx, root, &[a, b, c]), [0.0, 10.0, 30.0]);
        // Moving within a parent, and before itself.
        ctx.insert_before(root, c, Some(a)).unwrap();
        ctx.insert_before(root, c, Some(c)).unwrap();
        assert_eq!(x_positions(&mut ctx, root, &[c, a, b]), [0.0, 30.0, 40.0]);
        assert!(ctx.insert_before(root, a, Some(other)).is_err());
        assert!(ctx.insert_before(a, root, None).is_err());
        // Moving between parents.
        ctx.set_children(other, &[]).unwrap();
        ctx.insert_before(other, a, None).unwrap();
        assert_eq!(ctx.parent(a).unwrap(), Some(other));
        assert_eq!(x_positions(&mut ctx, root, &[c, b]), [0.0, 30.0]);
        ctx.detach(b).unwrap();
        assert_eq!(ctx.parent(b).unwrap(), None);
        ctx.detach(b).unwrap();
        assert_eq!(ctx.tree().unwrap().children(root.id), [c.id]);
        ctx.insert_before(root, b, Some(c)).unwrap();
        assert_eq!(x_positions(&mut ctx, root, &[b, c]), [0.0, 20.0]);
        // Content reaches the children's far edges, not the root's own size.
        assert_eq!(ctx.content_size(root).unwrap(), [50.0, 10.0]);
    }
    #[test]
    fn router_writes_box_model_and_order() {
        use crate::layout_props as p;
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(box_style(200.0, 80.0)).unwrap();
        let [a, b, c] = [10.0, 20.0, 30.0].map(|w| ctx.create_node(box_style(w, 10.0)).unwrap());
        ctx.set_children(root, &[a, b, c]).unwrap();
        ctx.apply(&[
            (a, ORDER, PropValue::Enum(1)),
            (root, p::PADDING_TOP + 3, PropValue::Number(5.0)),
            (root, BORDER_WIDTH, PropValue::Number(2.0)),
            (b, p::MARGIN_TOP + 3, PropValue::Number(3.0)),
        ])
        .unwrap();
        // Order sorts a after b and c; padding and border offset the content.
        assert_eq!(x_positions(&mut ctx, root, &[b, c, a]), [10.0, 30.0, 60.0]);
        let props = ctx
            .tree
            .as_ref()
            .unwrap()
            .props
            .get(&root.id)
            .map(|p| p.border_width);
        assert_eq!(props, Some(2.0));
        // Authored order survives: a new child goes after c, which sorts before a.
        let d = ctx.create_node(box_style(5.0, 10.0)).unwrap();
        ctx.insert_before(root, d, None).unwrap();
        assert_eq!(
            x_positions(&mut ctx, root, &[b, c, d, a]),
            [10.0, 30.0, 60.0, 65.0]
        );
        ctx.apply(&[(a, ORDER, PropValue::Unset)]).unwrap();
        assert_eq!(x_positions(&mut ctx, root, &[a, b]), [7.0, 20.0]);
        // A rejected batch writes nothing.
        let before = ctx.style(root).unwrap();
        assert!(
            ctx.apply(&[
                (root, 10, PropValue::Number(50.0)),
                (root, 4, PropValue::Number(0.5)),
            ])
            .is_err()
        );
        assert!(
            ctx.apply(&[(root, 10, PropValue::Text(Some("x")))])
                .is_err()
        );
        assert!(
            ctx.apply(&[(root, 10, PropValue::Number(f32::INFINITY))])
                .is_err()
        );
        assert_eq!(ctx.style(root).unwrap(), before);
        ctx.apply(&[
            (root, 27, PropValue::Enum(2)),
            (
                root,
                p::GRID_TEMPLATE_COLUMNS,
                PropValue::Text(Some("repeat(2, 1fr)")),
            ),
            (root, BORDER_WIDTH, PropValue::Unset),
            (root, p::PADDING_TOP + 3, PropValue::Unset),
        ])
        .unwrap();
        assert_eq!(ctx.style(root).unwrap().display, Display::Grid);
        assert_eq!(ctx.style(root).unwrap().border, <Style>::DEFAULT.border);
    }
}
