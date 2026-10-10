//! A [`Cascade`] applied to a [`Host`]'s tree, a [`LayoutContext`] or
//! another: the host sets each
//! element's names and states, and `restyle` matches what changed and
//! writes its layout declarations through the property router and reads its
//! paint declarations as typed writes for the host to apply to its own store.
//! What is neither, text for one, stays readable as resolved declarations.

use super::cascade::{Cascade, Computed, Element, States, Tree};
use super::layout::{Units, is_layout_property, layout_writes};
use super::paint::{PaintWrite, is_paint_property, paint_writes};
use super::quantity::PaintUnits;
use super::{Atom, MediaEnvironment, color};
use crate::context::{LayoutContext, Node, PropValue};
use blinc_layout::tree::LayoutNodeId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// A tree the cascade styles, its nodes by raw id, and where their layout
/// declarations go.
pub trait Host {
    fn live(&self, node: u64) -> bool;
    fn parent(&self, node: u64) -> Option<u64>;
    /// Every child, in order.
    fn children(&self, node: u64) -> Vec<u64>;
    /// Writes layout declarations through the property router. A host that
    /// lays out from the resolved declarations itself writes nothing.
    fn apply_layout(
        &mut self,
        node: u64,
        writes: &[(i32, PropValue<'_>)],
    ) -> Result<(), &'static str>;
    /// Whether layout declarations are read and written here; false for a host that applies them itself.
    fn lays_out(&self) -> bool {
        true
    }
    /// The context `Node`s of `take_paint` belong to; none for a host without one.
    fn context(&self) -> Option<u64> {
        None
    }
}

impl Host for LayoutContext {
    fn live(&self, node: u64) -> bool {
        self.node(node).is_ok()
    }
    fn parent(&self, node: u64) -> Option<u64> {
        let n = self.node(node).ok()?;
        LayoutContext::parent(self, n).ok().flatten().map(Node::raw)
    }
    fn children(&self, node: u64) -> Vec<u64> {
        self.node(node)
            .and_then(|n| LayoutContext::children(self, n))
            .map(|c| c.into_iter().map(Node::raw).collect())
            .unwrap_or_default()
    }
    fn apply_layout(
        &mut self,
        node: u64,
        writes: &[(i32, PropValue<'_>)],
    ) -> Result<(), &'static str> {
        let n = self.node(node)?;
        let staged: Vec<_> = writes.iter().map(|&(id, v)| (n, id, v)).collect();
        self.apply(&staged)
    }
    fn context(&self) -> Option<u64> {
        Some(self.id)
    }
}

/// A node, by its context `Node` or its raw id.
pub trait Key {
    fn key(self) -> u64;
}

impl Key for Node {
    fn key(self) -> u64 {
        self.raw()
    }
}

impl Key for u64 {
    fn key(self) -> u64 {
        self
    }
}

/// The cascade's view of a host: its nodes, and the elements described.
struct HostTree<'a, H: Host> {
    host: &'a H,
    elements: &'a FxHashMap<u64, Element>,
    /// What `:root` names; none for a forest, where it is any node with no parent.
    root: Option<u64>,
}

impl<H: Host> Tree for HostTree<'_, H> {
    type Node = u64;
    fn parent(&self, n: u64) -> Option<u64> {
        self.host.parent(n)
    }
    fn children(&self, n: u64) -> Vec<u64> {
        self.host.children(n)
    }
    fn element(&self, n: u64) -> Option<&Element> {
        self.elements.get(&n)
    }
    fn root(&self) -> Option<u64> {
        self.root
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
    /// What each node's paint was read from when last handed to the host, to
    /// write an unset for what it no longer has.
    paint_applied: FxHashMap<u64, PaintApplied>,
    /// Paint writes since the host last took them, node by node, in order.
    paint_out: Vec<(u64, Vec<PaintWrite>)>,
    /// The context of the host last restyled, whose `Node`s `take_paint` gives.
    context: u64,
    /// Nodes whose computed style changed since the host last took them.
    changed_out: Vec<u64>,
    /// States of nodes a selector began to test, since the host last took them.
    watched_out: Vec<(u64, u32)>,
    /// Whether paint is read at all: a host that draws from the resolved
    /// declarations itself never takes the writes.
    paint_output: bool,
}

/// The inputs of a node's paint writes: its declarations, and what relative
/// lengths and `currentcolor` in them mean.
#[derive(PartialEq)]
struct PaintApplied {
    declarations: Vec<(Atom, String)>,
    font_size: f64,
    color: Option<String>,
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
            paint_applied: FxHashMap::default(),
            paint_out: Vec::new(),
            paint_output: false,
            context: 0,
            changed_out: Vec::new(),
            watched_out: Vec::new(),
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
    pub fn set_element(&mut self, node: impl Key, mut element: Element) {
        let raw = node.key();
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
    pub fn moved(&mut self, node: impl Key) {
        self.changed.insert(node.key());
    }

    /// A child was placed under `parent` or removed from it: its children's
    /// places among their siblings may change.
    pub fn children_changed(&mut self, parent: impl Key) {
        self.reordered.insert(parent.key());
    }

    /// Sets `node`'s states; what tested a state that changed is restyled.
    pub fn set_states(&mut self, node: impl Key, states: States) {
        let node = node.key();
        let Some(e) = self.elements.get_mut(&node) else {
            return;
        };
        let flipped = e.states.0 ^ states.0;
        e.states = states;
        for bit in 0..32 {
            if flipped & (1 << bit) != 0
                && let Some(who) = self.dependents.get(&(node, 1 << bit))
            {
                self.stale.extend(who.iter().copied());
            }
        }
    }

    /// Forgets `node`, removed from the context.
    pub fn forget(&mut self, node: impl Key) {
        let raw = node.key();
        self.elements.remove(&raw);
        self.computed.remove(&raw);
        self.applied.remove(&raw);
        self.paint_applied.remove(&raw);
        self.untrack(raw);
    }

    /// Read paint declarations as typed writes, for [`Styles::take_paint`];
    /// off by default. Turning it on restyles every node.
    pub fn set_paint_output(&mut self, on: bool) {
        if self.paint_output != on {
            self.paint_output = on;
            self.paint_applied.clear();
            self.paint_out.clear();
            self.everything = true;
        }
    }

    /// Writes `node`'s paint again at the next `restyle`, though its
    /// declarations did not change: what was drawn over it is gone.
    pub fn repaint(&mut self, node: impl Key) {
        let raw = node.key();
        self.paint_applied.remove(&raw);
        self.stale.insert(raw);
    }

    /// The paint writes `restyle` has made since this was last called, by
    /// node, each node's in the order to apply them. Unsets are among them.
    pub fn take_paint(&mut self) -> Vec<(Node, Vec<PaintWrite>)> {
        let context = self.context;
        std::mem::take(&mut self.paint_out)
            .into_iter()
            .map(|(raw, w)| {
                let node = Node {
                    context,
                    id: LayoutNodeId::from_raw(raw),
                };
                (node, w)
            })
            .collect()
    }

    /// The nodes whose computed style `restyle` changed since this was last
    /// called, parents before their children: for a host that applies the
    /// resolved declarations itself.
    pub fn take_changed(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.changed_out)
    }

    /// As `take_changed`, at most `n` of them; the rest wait for the next call.
    pub fn take_changed_upto(&mut self, n: usize) -> Vec<u64> {
        let n = n.min(self.changed_out.len());
        self.changed_out.drain(..n).collect()
    }

    /// The states of nodes that selectors began to test since this was last
    /// called, as node and state bit: a host that tracks states lazily
    /// reports a change to one of these through `set_states`.
    pub fn take_watched(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.watched_out)
    }

    /// As `take_watched`, at most `n` of them; the rest wait for the next call.
    pub fn take_watched_upto(&mut self, n: usize) -> Vec<(u64, u32)> {
        let n = n.min(self.watched_out.len());
        self.watched_out.drain(..n).collect()
    }

    /// How many nodes `take_changed` would give.
    pub fn changed_count(&self) -> usize {
        self.changed_out.len()
    }

    /// What applies to `node` beyond layout, `var()`s resolved: paint and text declarations, by property name.
    pub fn resolved(&self, node: impl Key) -> impl Iterator<Item = (&str, &str)> {
        self.computed
            .get(&node.key())
            .into_iter()
            .flat_map(|c| c.resolved.iter())
            .map(|(k, v)| (self.cascade.str(*k), v.as_str()))
            .filter(|(k, _)| !is_layout_property(k))
    }

    /// How many nodes the last `restyle` styled.
    pub fn last_restyled(&self) -> usize {
        self.styled
    }

    pub fn computed(&self, node: impl Key) -> Option<&Computed> {
        self.computed.get(&node.key())
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
    pub fn restyle(&mut self, ctx: &mut LayoutContext, root: impl Key) -> Vec<String> {
        self.restyle_host(ctx, root)
    }

    /// `restyle` over any [`Host`]. A `root` of 0 restyles a forest: each node
    /// with no parent is a root.
    pub fn restyle_host<H: Host>(&mut self, host: &mut H, root: impl Key) -> Vec<String> {
        let root = Some(root.key()).filter(|&r| r != 0);
        let mut errors = Vec::new();
        self.styled = 0;
        if let Some(c) = host.context() {
            self.context = c;
        }
        let everything = std::mem::replace(&mut self.everything, false);
        let changed = std::mem::take(&mut self.changed);
        let reordered = std::mem::take(&mut self.reordered);
        let stale = std::mem::take(&mut self.stale);
        let mut work = Work::default();
        {
            let tree = HostTree {
                host: &*host,
                elements: &self.elements,
                root,
            };
            work.forest = root.is_none();
            if let Some(r) = root {
                work.depth.insert(r, 0);
            }
            if everything {
                match root {
                    Some(r) => work.subtree(&tree, r),
                    None => {
                        for &raw in self.elements.keys() {
                            if host.live(raw) {
                                work.push(&tree, raw);
                            }
                        }
                    }
                }
            } else {
                let position = self.cascade.tests_position();
                let above = self.cascade.tests_position_above();
                let has = self.cascade.uses_has();
                for raw in stale {
                    if host.live(raw) {
                        work.push(&tree, raw);
                    }
                }
                // Parents whose children changed: a changed node's, and those the host named.
                let mut parents: Vec<u64> = reordered
                    .into_iter()
                    .filter(|&raw| host.live(raw))
                    .collect();
                for raw in changed {
                    if !host.live(raw) {
                        continue;
                    }
                    work.subtree(&tree, raw);
                    if position || has {
                        parents.extend(tree.parent(raw));
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
            if !host.live(raw) {
                continue;
            }
            self.styled += 1;
            let (computed, deps) = {
                let tree = HostTree {
                    host: &*host,
                    elements: &self.elements,
                    root,
                };
                let parent = tree.parent(raw).and_then(|p| self.computed.get(&p));
                self.cascade.style(&tree, raw, parent)
            };
            // What it depends on, afresh.
            self.untrack(raw);
            for &key in &deps.states {
                let who = self.dependents.entry(key).or_default();
                if who.is_empty() {
                    self.watched_out.push(key);
                }
                who.insert(raw);
            }
            self.depends_on.insert(raw, deps.states);

            // Its layout declarations, and an unset for each it took before and no longer does.
            let (env, root_font_size) = self.cascade.environment();
            let units = Units {
                font_size: computed.font_size,
                root_font_size,
                viewport_width: env.width,
                viewport_height: env.height,
            };
            let lays_out = host.lays_out();
            let now: Vec<(Atom, &str)> = computed
                .resolved
                .iter()
                .filter(|(k, _)| lays_out && is_layout_property(self.cascade.str(*k)))
                .map(|(k, v)| (*k, v.as_str()))
                .collect();
            let before = self.applied.remove(&raw).unwrap_or_default();
            for &old in &before {
                if !now.iter().any(|(k, _)| *k == old)
                    && let Some(Ok(writes)) = layout_writes(self.cascade.str(old), None, &units)
                {
                    let _ = host.apply_layout(raw, &writes);
                }
            }
            for &(k, v) in &now {
                let name = self.cascade.str(k);
                match layout_writes(name, Some(v), &units) {
                    Some(Ok(writes)) => {
                        if let Err(e) = host.apply_layout(raw, &writes) {
                            errors.push(format!("{name}: {v}: {e}"));
                        }
                    }
                    Some(Err(e)) => errors.push(format!("{name}: {v}: {e}")),
                    None => {}
                }
            }
            self.applied
                .insert(raw, now.iter().map(|(k, _)| *k).collect());
            self.read_paint(raw, &computed, units, &mut errors);
            // What its children inherit changed: they are restyled too, after it.
            let inherits = |k: &Atom| self.cascade.inherits(*k);
            let old = self.computed.get(&raw);
            let passes_same = old.is_some_and(|old| {
                old.values
                    .iter()
                    .filter(|(k, _)| inherits(k))
                    .eq(computed.values.iter().filter(|(k, _)| inherits(k)))
            });
            if old != Some(&computed) {
                self.changed_out.push(raw);
            }
            if !passes_same {
                for c in host.children(raw) {
                    work.push_at(c, depth + 1);
                }
            }
            self.computed.insert(raw, computed);
        }
        errors
    }
}

impl Styles {
    /// `node`'s paint as typed writes, if it differs from what was handed to
    /// the host: each declaration it has now, and an unset for each it had.
    fn read_paint(
        &mut self,
        raw: u64,
        computed: &Computed,
        units: Units,
        errors: &mut Vec<String>,
    ) {
        if !self.paint_output {
            return;
        }
        let declared: Vec<(&str, &str)> = computed
            .resolved
            .iter()
            .map(|(k, v)| (self.cascade.str(*k), v.as_str()))
            .filter(|(k, _)| is_paint_property(k))
            .collect();
        let own_color = computed
            .values
            .iter()
            .find(|(k, _)| self.cascade.str(*k) == "color")
            .map(|(_, v)| v.clone());
        let now = PaintApplied {
            declarations: computed
                .resolved
                .iter()
                .filter(|(k, _)| is_paint_property(self.cascade.str(*k)))
                .cloned()
                .collect(),
            font_size: computed.font_size,
            color: own_color.clone(),
        };
        let before = self.paint_applied.get(&raw);
        if before == Some(&now) || (before.is_none() && now.declarations.is_empty()) {
            return;
        }
        let paint_units = PaintUnits {
            font_size: units.font_size,
            root_font_size: units.root_font_size,
            viewport_width: units.viewport_width,
            viewport_height: units.viewport_height,
            color: own_color.and_then(|c| color::parse(&c, None).ok()),
            declared: &declared,
        };
        let mut writes = Vec::new();
        if let Some(old) = before {
            for (k, _) in &old.declarations {
                let name = self.cascade.str(*k);
                if !declared.iter().any(|(n, _)| *n == name)
                    && let Some(Ok(w)) = paint_writes(name, None, &paint_units)
                {
                    writes.extend(w);
                }
            }
        }
        for (name, value) in &declared {
            match paint_writes(name, Some(value), &paint_units) {
                Some(Ok(w)) => writes.extend(w),
                Some(Err(e)) => errors.push(format!("{name}: {value}: {e}")),
                None => {}
            }
        }
        if now.declarations.is_empty() {
            self.paint_applied.remove(&raw);
        } else {
            self.paint_applied.insert(raw, now);
        }
        if !writes.is_empty() {
            self.paint_out.push((raw, writes));
        }
    }
}

/// Nodes to restyle, shallowest first, each once.
#[derive(Default)]
struct Work {
    heap: BinaryHeap<Reverse<(u32, u64)>>,
    queued: FxHashSet<u64>,
    /// Depth under the root, of the nodes met so far.
    depth: FxHashMap<u64, u32>,
    /// No one root: a node with no parent is at depth 0.
    forest: bool,
}

impl Work {
    fn push_at(&mut self, raw: u64, depth: u32) {
        self.depth.insert(raw, depth);
        if self.queued.insert(raw) {
            self.heap.push(Reverse((depth, raw)));
        }
    }

    /// `n`'s depth under the root, or none when it is not under it.
    fn depth_of<T: Tree<Node = u64>>(&mut self, tree: &T, n: u64) -> Option<u32> {
        let mut path = Vec::new();
        let mut at = n;
        let base = loop {
            if let Some(&d) = self.depth.get(&at) {
                break d;
            }
            match tree.parent(at) {
                Some(p) => {
                    path.push(at);
                    at = p;
                }
                None if self.forest => {
                    self.depth.insert(at, 0);
                    break 0;
                }
                None => return None,
            }
        };
        for (i, raw) in path.iter().rev().enumerate() {
            self.depth.insert(*raw, base + 1 + i as u32);
        }
        self.depth.get(&n).copied()
    }

    fn push<T: Tree<Node = u64>>(&mut self, tree: &T, n: u64) {
        if let Some(d) = self.depth_of(tree, n) {
            self.push_at(n, d);
        }
    }

    fn subtree<T: Tree<Node = u64>>(&mut self, tree: &T, n: u64) {
        let Some(d) = self.depth_of(tree, n) else {
            return;
        };
        let mut stack = vec![(n, d)];
        while let Some((x, d)) = stack.pop() {
            self.push_at(x, d);
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
