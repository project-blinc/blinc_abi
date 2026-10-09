//! A [`Cascade`] applied to a [`LayoutContext`]: the host sets each
//! element's names and states, and `restyle` matches what changed and
//! writes its layout declarations through the property router. What is not
//! layout, paint and text, stays readable as resolved declarations for the
//! host to draw with.

use super::cascade::{Cascade, Computed, Element, States, Tree};
use super::layout::{Units, is_layout_property, layout_writes};
use super::{Atom, MediaEnvironment};
use crate::context::{LayoutContext, Node};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// The cascade's view of a context: its nodes, and the elements the host described.
struct ContextTree<'a> {
    ctx: &'a LayoutContext,
    elements: &'a FxHashMap<u64, Element>,
    root: Node,
}

impl Tree for ContextTree<'_> {
    type Node = Node;
    fn parent(&self, n: Node) -> Option<Node> {
        self.ctx.parent(n).ok().flatten()
    }
    fn children(&self, n: Node) -> Vec<Node> {
        self.ctx.children(n).unwrap_or_default()
    }
    fn element(&self, n: Node) -> Option<&Element> {
        self.elements.get(&n.raw())
    }
    fn root(&self) -> Option<Node> {
        Some(self.root)
    }
}

/// A cascade over one context, and what it last applied there.
pub struct Styles {
    cascade: Cascade,
    elements: FxHashMap<u64, Element>,
    computed: FxHashMap<u64, Computed>,
    /// The layout properties each node took from the cascade, to unset those it no longer takes.
    applied: FxHashMap<u64, Vec<Atom>>,
    /// Who depends on which state of which node: restyled when it changes.
    dependents: FxHashMap<(u64, u32), FxHashSet<u64>>,
    /// What each node depends on, to forget it when the node is restyled.
    depends_on: FxHashMap<u64, Vec<(u64, u32)>>,
    /// Nodes whose names, place or sheets changed: each is restyled with what
    /// is under it and after it, which selectors and inheritance reach.
    changed: FxHashSet<u64>,
    /// Parents a child was placed in or removed from.
    reordered: FxHashSet<u64>,
    /// Nodes to restyle alone: a state they depend on changed.
    stale: FxHashSet<u64>,
    everything: bool,
    /// How many nodes the last `restyle` styled.
    styled: usize,
}

impl Default for Styles {
    fn default() -> Self {
        Self::new()
    }
}

impl Styles {
    pub fn new() -> Self {
        Styles {
            cascade: Cascade::new(),
            elements: FxHashMap::default(),
            computed: FxHashMap::default(),
            applied: FxHashMap::default(),
            dependents: FxHashMap::default(),
            depends_on: FxHashMap::default(),
            changed: FxHashSet::default(),
            reordered: FxHashSet::default(),
            stale: FxHashSet::default(),
            everything: true,
            styled: 0,
        }
    }

    pub fn cascade(&self) -> &Cascade {
        &self.cascade
    }

    /// The cascade, to add or remove sheets or set the theme: every node is restyled.
    pub fn cascade_mut(&mut self) -> &mut Cascade {
        self.everything = true;
        &mut self.cascade
    }

    pub fn intern(&mut self, name: &str) -> Atom {
        self.cascade.intern(name)
    }

    pub fn set_environment(&mut self, env: MediaEnvironment) {
        self.cascade.set_environment(env);
        self.everything = true;
    }

    /// Describes `node` as an element; its states are kept. A change to
    /// names no selector tests on another element, or to its own
    /// declarations alone, restyles it alone; what it passes to its children
    /// reaches them if it changed.
    pub fn set_element(&mut self, node: Node, mut element: Element) {
        let raw = node.raw();
        match self.elements.get(&raw) {
            None => {
                self.changed.insert(raw);
            }
            Some(old) => {
                element.states = old.states;
                let names = changed_names(old, &element);
                if names.iter().any(|&a| self.cascade.reaches(a)) {
                    self.changed.insert(raw);
                } else if !names.is_empty() || old.inline != element.inline {
                    self.stale.insert(raw);
                }
            }
        }
        self.elements.insert(raw, element);
    }

    /// `node` was placed under a new parent: what it and its descendants
    /// match may change. Give the parent it left to `children_changed`; a
    /// move among the same siblings is `children_changed` alone.
    pub fn moved(&mut self, node: Node) {
        self.changed.insert(node.raw());
    }

    /// A child was placed under `parent` or removed from it: its children's
    /// places among their siblings may change.
    pub fn children_changed(&mut self, parent: Node) {
        self.reordered.insert(parent.raw());
    }

    /// Sets `node`'s states; what tested a state that changed is restyled.
    pub fn set_states(&mut self, node: Node, states: States) {
        let Some(e) = self.elements.get_mut(&node.raw()) else {
            return;
        };
        let flipped = e.states.0 ^ states.0;
        e.states = states;
        for bit in 0..32 {
            if flipped & (1 << bit) != 0
                && let Some(who) = self.dependents.get(&(node.raw(), 1 << bit))
            {
                self.stale.extend(who.iter().copied());
            }
        }
    }

    /// Forgets `node`, removed from the context.
    pub fn forget(&mut self, node: Node) {
        let raw = node.raw();
        self.elements.remove(&raw);
        self.computed.remove(&raw);
        self.applied.remove(&raw);
        self.untrack(raw);
    }

    /// What applies to `node` beyond layout, `var()`s resolved: paint and text declarations, by property name.
    pub fn resolved(&self, node: Node) -> impl Iterator<Item = (&str, &str)> {
        self.computed
            .get(&node.raw())
            .into_iter()
            .flat_map(|c| c.resolved.iter())
            .map(|(k, v)| (self.cascade.str(*k), v.as_str()))
            .filter(|(k, _)| !is_layout_property(k))
    }

    /// How many nodes the last `restyle` styled.
    pub fn last_restyled(&self) -> usize {
        self.styled
    }

    pub fn computed(&self, node: Node) -> Option<&Computed> {
        self.computed.get(&node.raw())
    }

    fn untrack(&mut self, raw: u64) {
        for key in self.depends_on.remove(&raw).unwrap_or_default() {
            if let Some(set) = self.dependents.get_mut(&key) {
                set.remove(&raw);
                if set.is_empty() {
                    self.dependents.remove(&key);
                }
            }
        }
    }

    /// Restyles what changed under `root`, parents first, and writes its
    /// layout declarations to `ctx`. Returns the declarations it could not
    /// apply, as `property: value: reason`; the rest still apply.
    pub fn restyle(&mut self, ctx: &mut LayoutContext, root: Node) -> Vec<String> {
        let mut errors = Vec::new();
        self.styled = 0;
        let everything = std::mem::replace(&mut self.everything, false);
        let changed = std::mem::take(&mut self.changed);
        let reordered = std::mem::take(&mut self.reordered);
        let stale = std::mem::take(&mut self.stale);
        let mut work = Work::default();
        {
            let tree = ContextTree {
                ctx,
                elements: &self.elements,
                root,
            };
            work.depth.insert(root.raw(), 0);
            if everything {
                work.subtree(&tree, root);
            } else {
                let position = self.cascade.tests_position();
                let above = self.cascade.tests_position_above();
                let has = self.cascade.uses_has();
                for raw in stale {
                    if let Ok(n) = ctx.node(raw) {
                        work.push(&tree, n);
                    }
                }
                // Parents whose children changed: a changed node's, and those the host named.
                let mut parents: Vec<Node> = reordered
                    .into_iter()
                    .filter_map(|raw| ctx.node(raw).ok())
                    .collect();
                for raw in changed {
                    let Ok(n) = ctx.node(raw) else { continue };
                    work.subtree(&tree, n);
                    if position || has {
                        parents.extend(tree.parent(n));
                    }
                }
                for p in parents {
                    // Its children's places among their siblings, and whether it is :empty.
                    if position {
                        work.push(&tree, p);
                        for c in tree.children(p) {
                            if above {
                                work.subtree(&tree, c);
                            } else {
                                work.push(&tree, c);
                            }
                        }
                    }
                    // A :has() above or beside it may answer differently: its ancestors and their children, each alone.
                    if has {
                        let mut at = Some(p);
                        while let Some(x) = at {
                            work.push(&tree, x);
                            for c in tree.children(x) {
                                work.push(&tree, c);
                            }
                            at = tree.parent(x);
                        }
                    }
                }
            }
        }

        // Shallowest first, so a child inherits what its parent has now.
        while let Some(Reverse((depth, raw))) = work.heap.pop() {
            let Ok(n) = ctx.node(raw) else { continue };
            self.styled += 1;
            let (computed, deps) = {
                let tree = ContextTree {
                    ctx,
                    elements: &self.elements,
                    root,
                };
                let parent = tree.parent(n).and_then(|p| self.computed.get(&p.raw()));
                self.cascade.style(&tree, n, parent)
            };
            // What it depends on, afresh.
            self.untrack(raw);
            let keys: Vec<(u64, u32)> =
                deps.states.iter().map(|(m, bit)| (m.raw(), *bit)).collect();
            for &key in &keys {
                self.dependents.entry(key).or_default().insert(raw);
            }
            self.depends_on.insert(raw, keys);

            // Its layout declarations, and an unset for each it took before and no longer does.
            let (env, root_font_size) = self.cascade.environment();
            let units = Units {
                font_size: computed.font_size,
                root_font_size,
                viewport_width: env.width,
                viewport_height: env.height,
            };
            let now: Vec<(Atom, &str)> = computed
                .resolved
                .iter()
                .filter(|(k, _)| is_layout_property(self.cascade.str(*k)))
                .map(|(k, v)| (*k, v.as_str()))
                .collect();
            let before = self.applied.remove(&raw).unwrap_or_default();
            for &old in &before {
                if !now.iter().any(|(k, _)| *k == old)
                    && let Some(Ok(writes)) = layout_writes(self.cascade.str(old), None, &units)
                {
                    let staged: Vec<_> = writes.into_iter().map(|(id, v)| (n, id, v)).collect();
                    let _ = ctx.apply(&staged);
                }
            }
            for &(k, v) in &now {
                let name = self.cascade.str(k);
                match layout_writes(name, Some(v), &units) {
                    Some(Ok(writes)) => {
                        let staged: Vec<_> = writes.into_iter().map(|(id, v)| (n, id, v)).collect();
                        if let Err(e) = ctx.apply(&staged) {
                            errors.push(format!("{name}: {v}: {e}"));
                        }
                    }
                    Some(Err(e)) => errors.push(format!("{name}: {v}: {e}")),
                    None => {}
                }
            }
            self.applied
                .insert(raw, now.iter().map(|(k, _)| *k).collect());
            // What its children inherit changed: they are restyled too, after it.
            let inherits = |k: &Atom| self.cascade.inherits(*k);
            let passes_same = self.computed.get(&raw).is_some_and(|old| {
                old.values
                    .iter()
                    .filter(|(k, _)| inherits(k))
                    .eq(computed.values.iter().filter(|(k, _)| inherits(k)))
            });
            if !passes_same {
                for c in ctx.children(n).unwrap_or_default() {
                    work.push_at(c.raw(), depth + 1);
                }
            }
            self.computed.insert(raw, computed);
        }
        errors
    }
}

/// Nodes to restyle, shallowest first, each once.
#[derive(Default)]
struct Work {
    heap: BinaryHeap<Reverse<(u32, u64)>>,
    queued: FxHashSet<u64>,
    /// Depth under the root, of the nodes met so far.
    depth: FxHashMap<u64, u32>,
}

impl Work {
    fn push_at(&mut self, raw: u64, depth: u32) {
        self.depth.insert(raw, depth);
        if self.queued.insert(raw) {
            self.heap.push(Reverse((depth, raw)));
        }
    }

    /// `n`'s depth under the root, or none when it is not under it.
    fn depth_of(&mut self, tree: &ContextTree, n: Node) -> Option<u32> {
        let mut path = Vec::new();
        let mut at = n;
        let base = loop {
            if let Some(&d) = self.depth.get(&at.raw()) {
                break d;
            }
            path.push(at.raw());
            at = tree.parent(at)?;
        };
        for (i, raw) in path.iter().rev().enumerate() {
            self.depth.insert(*raw, base + 1 + i as u32);
        }
        self.depth.get(&n.raw()).copied()
    }

    fn push(&mut self, tree: &ContextTree, n: Node) {
        if let Some(d) = self.depth_of(tree, n) {
            self.push_at(n.raw(), d);
        }
    }

    fn subtree(&mut self, tree: &ContextTree, n: Node) {
        let Some(d) = self.depth_of(tree, n) else {
            return;
        };
        let mut stack = vec![(n, d)];
        while let Some((x, d)) = stack.pop() {
            self.push_at(x.raw(), d);
            stack.extend(tree.children(x).into_iter().map(|c| (c, d + 1)));
        }
    }
}

/// The names that differ between two descriptions of an element: types, id,
/// classes, and attributes added, removed or given another value.
fn changed_names(old: &Element, new: &Element) -> Vec<Atom> {
    let mut out = Vec::new();
    let diff = |a: &[Atom], b: &[Atom], out: &mut Vec<Atom>| {
        out.extend(a.iter().filter(|x| !b.contains(x)));
        out.extend(b.iter().filter(|x| !a.contains(x)));
    };
    diff(&old.types, &new.types, &mut out);
    diff(&old.classes, &new.classes, &mut out);
    if old.id != new.id {
        out.extend(old.id);
        out.extend(new.id);
    }
    for (k, v) in &old.attributes {
        if !new.attributes.iter().any(|(nk, nv)| nk == k && nv == v) {
            out.push(*k);
        }
    }
    for (k, _) in &new.attributes {
        if !old.attributes.iter().any(|(ok, _)| ok == k) {
            out.push(*k);
        }
    }
    out
}
