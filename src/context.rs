//! Owned layout contexts for language adapters. No host GC or process-global tree.
use blinc_layout::tree::{LayoutNodeId, LayoutTree};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
pub use taffy::prelude::{AvailableSpace, Size, Style};

static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);

/// A generation-checked node, valid only in its originating context.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Node {
    context: u64,
    id: LayoutNodeId,
}

/// Each adapter owns a context and serializes access to it on its host thread.
/// Styles are Taffy's complete styles; SDKs choose their authored style surface.
pub struct LayoutContext {
    id: u64,
    tree: Option<crate::tree::Tree>,
    parents: HashMap<LayoutNodeId, LayoutNodeId>,
    bounds: HashMap<LayoutNodeId, [f32; 4]>,
}

type Result<T> = std::result::Result<T, &'static str>;

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
            parents: HashMap::new(),
            bounds: HashMap::new(),
        }
    }

    fn tree(&self) -> Result<&LayoutTree> {
        self.tree
            .as_ref()
            .map(|tree| &tree.layout)
            .ok_or("Layout context is disposed")
    }
    fn check(&self, node: Node) -> Result<LayoutNodeId> {
        let tree = self.tree()?;
        if node.context != self.id {
            return Err("Node belongs to another layout context");
        }
        if !tree.node_exists(node.id) {
            return Err("Layout node is removed");
        }
        Ok(node.id)
    }

    pub fn create_node(&mut self, style: Style) -> Result<Node> {
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
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
        self.tree.as_mut().unwrap().set_style(id, style);
        self.bounds.clear();
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
        let tree = self.tree.as_mut().unwrap();
        // Rebuild each previous parent's child list only once for a bulk move.
        let old_parents: HashSet<_> = ids
            .iter()
            .filter_map(|id| self.parents.get(id).copied())
            .filter(|&id| id != parent)
            .collect();
        for old_parent in old_parents {
            let siblings = tree
                .children(old_parent)
                .into_iter()
                .filter(|id| !seen.contains(id))
                .collect();
            tree.replace_children(old_parent, siblings);
        }
        for old in tree.replace_children(parent, ids) {
            self.parents.remove(&old);
        }
        for child in children {
            self.parents.insert(child.id, parent);
        }
        self.bounds.clear();
        Ok(())
    }

    pub fn remove(&mut self, node: Node) -> Result<()> {
        let id = self.check(node)?;
        let tree = self.tree.as_mut().unwrap();
        let mut nodes = vec![id];
        let mut index = 0;
        while index < nodes.len() {
            nodes.extend(tree.children(nodes[index]));
            index += 1;
        }
        // Iterative removal also handles deeply nested authored trees.
        for id in nodes.into_iter().rev() {
            self.parents.remove(&id);
            tree.remove_node(id);
            tree.forget(id);
        }
        self.bounds.clear();
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
        self.tree.as_mut().unwrap().compute_layout(
            id,
            Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
        );
        // Resolve absolute coordinates once, not once per queried node and ancestor.
        let tree = self.tree.as_ref().unwrap();
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
        ctx.dispose();
        ctx.dispose();
        assert!(ctx.create_node(Style::default()).is_err());
    }
}
