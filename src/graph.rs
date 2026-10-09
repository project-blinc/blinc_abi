//! Owned Blinc dependency graph. Host values stay in the language adapter;
//! native signals/deriveds carry unit values and own dependency scheduling.
use blinc_core::reactive::{Derived, Effect, EffectId, ReactiveGraph, Signal};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

static NEXT_GRAPH: AtomicU64 = AtomicU64::new(1);
type Result<T> = std::result::Result<T, &'static str>;
type Command = Box<dyn FnOnce(&mut ReactiveGraph)>;

#[derive(Clone, Copy)]
pub struct SignalKey {
    context: u64,
    native: Signal<()>,
}
#[derive(Clone, Copy)]
pub struct ComputedKey {
    context: u64,
    native: Derived<()>,
}
#[derive(Clone)]
pub struct EffectKey {
    context: u64,
    native: Rc<Cell<Option<Effect>>>,
    alive: Rc<Cell<bool>>,
}

/// An effect whose body the host runs: due effects are taken by tag and run
/// between `begin_effect` and `end_effect`.
#[derive(Clone, Copy)]
pub struct HostEffectKey {
    context: u64,
    native: Effect,
}

#[derive(Clone, Copy)]
struct Active {
    id: u64,
    graph: *const ReactiveGraph,
    computed: bool,
}
thread_local! { static ACTIVE: Cell<Option<Active>> = const { Cell::new(None) }; }
struct ReadGuard(Option<Active>);
impl Drop for ReadGuard {
    fn drop(&mut self) {
        ACTIVE.with(|slot| slot.set(self.0));
    }
}

fn evaluate<R>(id: u64, graph: &ReactiveGraph, computed: bool, run: impl FnOnce() -> R) -> R {
    let previous = ACTIVE.with(|slot| {
        slot.replace(Some(Active {
            id,
            graph,
            computed,
        }))
    });
    let _guard = ReadGuard(previous);
    run()
}

/// Single-thread owned context. Rc and RefCell deliberately prevent Send/Sync.
/// Mutations made by effects are deferred until the running callback returns.
pub struct GraphContext {
    id: u64,
    graph: RefCell<Option<ReactiveGraph>>,
    pending: RefCell<VecDeque<Command>>,
    draining: Cell<bool>,
    disposed: Arc<AtomicBool>,
    /// The host's tag for each host effect, which due effects are reported by.
    host_tags: RefCell<HashMap<EffectId, u32>>,
    /// Host effect scopes now open, innermost last. Commands wait while any is.
    host_scopes: RefCell<Vec<EffectId>>,
}
#[derive(Debug, Clone, Copy)]
pub struct GraphStats {
    pub signals: usize,
    pub computeds: usize,
    pub effects: usize,
}
impl Default for GraphContext {
    fn default() -> Self {
        Self::new()
    }
}
impl GraphContext {
    pub fn new() -> Self {
        Self {
            id: NEXT_GRAPH.fetch_add(1, Ordering::Relaxed),
            graph: RefCell::new(Some(ReactiveGraph::new())),
            pending: RefCell::new(VecDeque::new()),
            draining: Cell::new(false),
            disposed: Arc::new(AtomicBool::new(false)),
            host_tags: RefCell::new(HashMap::new()),
            host_scopes: RefCell::new(Vec::new()),
        }
    }
    pub fn is_disposed(&self) -> bool {
        self.disposed.load(Ordering::Relaxed)
    }
    fn active(&self) -> Result<Option<Active>> {
        if self.is_disposed() {
            return Err("Reactive context is disposed");
        }
        let active = ACTIVE.with(Cell::get);
        if active.is_some_and(|a| a.id != self.id) {
            return Err("Reactive dependencies cannot cross contexts");
        }
        Ok(active)
    }
    fn own(&self, id: u64) -> Result<()> {
        if self.id != id {
            Err("Reactive handle belongs to another context")
        } else {
            Ok(())
        }
    }
    fn read<R>(&self, run: impl FnOnce(&ReactiveGraph) -> R) -> Result<R> {
        if let Some(active) = self.active()? {
            // SAFETY: evaluate's guard confines this pointer to a callback on
            // this thread; the graph's exclusive outer call remains live. Only
            // ReactiveGraph's shared, reentrant read methods are used here.
            return Ok(run(unsafe { &*active.graph }));
        }
        let graph = self.graph.borrow();
        Ok(run(graph.as_ref().ok_or("Reactive context is disposed")?))
    }
    pub fn is_evaluating(&self) -> bool {
        ACTIVE.with(|slot| slot.get().is_some_and(|a| a.id == self.id))
    }
    pub fn assert_mutable(&self) -> Result<()> {
        if self.active()?.is_some_and(|a| a.computed) {
            return Err("Cannot mutate reactive state from a computed callback");
        }
        Ok(())
    }
    fn enqueue(&self, command: Command) -> Result<()> {
        if self.active()?.is_some_and(|a| a.computed) {
            return Err("Cannot mutate reactive state from a computed callback");
        }
        self.pending.borrow_mut().push_back(command);
        self.flush()
    }
    fn flush(&self) -> Result<()> {
        if ACTIVE.with(Cell::get).is_some()
            || !self.host_scopes.borrow().is_empty()
            || self.draining.replace(true)
        {
            return Ok(());
        }
        struct DrainGuard<'a>(&'a Cell<bool>);
        impl Drop for DrainGuard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _guard = DrainGuard(&self.draining);
        let mut waves = 0;
        while !self.pending.borrow().is_empty() && !self.is_disposed() {
            waves += 1;
            if waves > 1024 {
                self.pending.borrow_mut().clear();
                return Err("Reactive update cycle exceeded 1024 flush waves");
            }
            let count = self.pending.borrow().len();
            let mut slot = self.graph.borrow_mut();
            let graph = slot.as_mut().ok_or("Reactive context is disposed")?;
            graph.batch_start();
            for _ in 0..count {
                let command = self.pending.borrow_mut().pop_front().unwrap();
                command(graph);
            }
            graph.batch_end();
            graph.take_dirty_derived();
        }
        if self.is_disposed() {
            self.pending.borrow_mut().clear();
            self.graph.borrow_mut().take();
        }
        Ok(())
    }
    pub fn signal(&self) -> Result<SignalKey> {
        self.read(|graph| SignalKey {
            context: self.id,
            native: graph.create_signal(()),
        })
    }
    pub fn track(&self, key: SignalKey) -> Result<()> {
        self.own(key.context)?;
        self.read(|g| g.get(key.native))?
            .ok_or("Signal is disposed")
    }
    pub fn check_signal(&self, key: SignalKey) -> Result<()> {
        self.own(key.context)?;
        self.read(|g| g.get_untracked(key.native))?
            .ok_or("Signal is disposed")
    }
    pub fn notify(&self, key: SignalKey) -> Result<()> {
        self.check_signal(key)?;
        self.enqueue(Box::new(move |g| g.set(key.native, ())))
    }
    pub fn remove_signal(&self, key: SignalKey) -> Result<()> {
        self.own(key.context)?;
        self.enqueue(Box::new(move |g| {
            g.dispose_signal(key.native.id());
        }))
    }
    pub fn computed(&self, callback: impl Fn() + Send + 'static) -> Result<ComputedKey> {
        let id = self.id;
        let disposed = self.disposed.clone();
        self.read(move |graph| ComputedKey {
            context: id,
            native: graph.create_derived(move |g| {
                if !disposed.load(Ordering::Relaxed) {
                    evaluate(id, g, true, &callback);
                }
            }),
        })
    }
    pub fn track_computed(&self, key: ComputedKey) -> Result<()> {
        self.own(key.context)?;
        let value = if let Some(active) = self.active()? {
            // Same lifetime/read-only contract as read().
            unsafe { (&*active.graph).read_derived_in_flight(key.native) }
        } else {
            self.graph
                .borrow_mut()
                .as_mut()
                .ok_or("Reactive context is disposed")?
                .get_derived(key.native)
        };
        self.flush()?;
        value.ok_or("Computed is disposed or contains a dependency cycle")
    }
    pub fn remove_computed(&self, key: ComputedKey) -> Result<()> {
        self.own(key.context)?;
        self.enqueue(Box::new(move |g| {
            g.dispose_derived(key.native.id());
        }))
    }
    pub fn effect(&self, callback: impl Fn() + Send + 'static) -> Result<EffectKey> {
        let id = self.id;
        let disposed = self.disposed.clone();
        let key = EffectKey {
            context: id,
            native: Rc::new(Cell::new(None)),
            alive: Rc::new(Cell::new(true)),
        };
        let slot = key.native.clone();
        let alive = key.alive.clone();
        self.enqueue(Box::new(move |graph| {
            if alive.get() {
                slot.set(Some(graph.create_effect(move |g| {
                    if !disposed.load(Ordering::Relaxed) {
                        evaluate(id, g, false, &callback);
                    }
                })));
            }
        }))?;
        Ok(key)
    }
    pub fn remove_effect(&self, key: &EffectKey) -> Result<()> {
        self.own(key.context)?;
        self.assert_mutable()?;
        key.alive.set(false);
        let slot = key.native.clone();
        self.enqueue(Box::new(move |graph| {
            if let Some(effect) = slot.take() {
                graph.dispose_effect(effect);
            }
        }))
    }
    /// The graph, borrowed for an edit made outside every graph callback.
    fn edit<R>(&self, run: impl FnOnce(&mut ReactiveGraph) -> R) -> Result<R> {
        if self.active()?.is_some() {
            return Err("Host effects cannot be used inside a graph callback");
        }
        let mut slot = self
            .graph
            .try_borrow_mut()
            .map_err(|_| "Reactive graph is busy")?;
        Ok(run(slot.as_mut().ok_or("Reactive context is disposed")?))
    }

    /// A host effect, reported by `tag` when due. It is due at once, as an
    /// effect runs at once.
    pub fn host_effect(&self, tag: u32) -> Result<HostEffectKey> {
        let native = self.edit(|graph| graph.create_host_effect())?;
        self.host_tags.borrow_mut().insert(native.id(), tag);
        Ok(HostEffectKey {
            context: self.id,
            native,
        })
    }

    /// Append the tags of the host effects now due to `out`, in the order they
    /// became due. Each is handed out once until it has run.
    pub fn take_due_host_effects(&self, out: &mut Vec<u32>) -> Result<()> {
        let due = self.edit(|graph| graph.take_due_host_effects())?;
        let tags = self.host_tags.borrow();
        out.extend(due.iter().filter_map(|id| tags.get(id).copied()));
        Ok(())
    }

    /// Track what the host reads as the effect's dependencies until
    /// `end_effect`. Commands such as signal writes wait until the outermost
    /// scope ends. False if the effect was removed.
    pub fn begin_effect(&self, key: HostEffectKey) -> Result<bool> {
        self.own(key.context)?;
        let id = key.native.id();
        if !self.host_tags.borrow().contains_key(&id) {
            return Ok(false);
        }
        let begun = self.edit(|graph| graph.begin_effect(id))?;
        if begun {
            self.host_scopes.borrow_mut().push(id);
        }
        Ok(begun)
    }

    /// Close the effect's scope, and any opened inside it that were not
    /// closed. At the outermost scope the waiting commands are applied, so an
    /// effect that wrote what it read becomes due again.
    pub fn end_effect(&self, key: HostEffectKey) -> Result<()> {
        self.own(key.context)?;
        let id = key.native.id();
        let Some(at) = self
            .host_scopes
            .borrow()
            .iter()
            .rposition(|&open| open == id)
        else {
            return Ok(());
        };
        if self.is_disposed() {
            self.host_scopes.borrow_mut().truncate(at);
            return self.flush();
        }
        self.edit(|graph| graph.end_effect(id))?;
        self.host_scopes.borrow_mut().truncate(at);
        self.flush()
    }

    pub fn remove_host_effect(&self, key: HostEffectKey) -> Result<()> {
        self.own(key.context)?;
        self.assert_mutable()?;
        if self
            .host_tags
            .borrow_mut()
            .remove(&key.native.id())
            .is_none()
        {
            return Ok(());
        }
        let effect = key.native;
        self.enqueue(Box::new(move |graph| graph.dispose_effect(effect)))
    }

    pub fn begin_batch(&self) -> Result<()> {
        self.assert_mutable()?;
        self.read(|graph| graph.batch_start())
    }
    pub fn end_batch(&self) -> Result<()> {
        self.enqueue(Box::new(|graph| graph.batch_end()))
    }
    pub fn stats(&self) -> Result<GraphStats> {
        self.read(|g| {
            let s = g.stats();
            GraphStats {
                signals: s.signal_count,
                computeds: s.derived_count,
                effects: s.effect_count,
            }
        })
    }
    pub fn dispose(&self) {
        self.disposed.store(true, Ordering::Relaxed);
        if !ACTIVE.with(|slot| slot.get().is_some_and(|a| a.id == self.id)) && !self.draining.get()
        {
            self.pending.borrow_mut().clear();
            self.graph.borrow_mut().take();
        }
        self.host_tags.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    #[test]
    fn contexts_are_independent_and_batches_deduplicate_effects() {
        let graph = GraphContext::new();
        let other = GraphContext::new();
        let signal = graph.signal().unwrap();
        assert!(other.track(signal).is_err());
        let runs = Arc::new(AtomicUsize::new(0));
        let out = runs.clone();
        // The callback uses Blinc's in-flight getter and owns no host pointers.
        let native = signal.native;
        let effect = graph
            .effect(move || {
                native.get();
                out.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
        assert_eq!(runs.load(Ordering::Relaxed), 1);
        graph.begin_batch().unwrap();
        graph.notify(signal).unwrap();
        graph.notify(signal).unwrap();
        assert_eq!(runs.load(Ordering::Relaxed), 1);
        graph.end_batch().unwrap();
        assert_eq!(runs.load(Ordering::Relaxed), 2);
        graph.remove_effect(&effect).unwrap();
        graph.notify(signal).unwrap();
        assert_eq!(runs.load(Ordering::Relaxed), 2);
        graph.dispose();
        graph.dispose();
        assert!(graph.track(signal).is_err());
    }

    /// Run every due host effect as a host would: take the due tags, run each
    /// between begin and end, and repeat until none is due.
    fn run_due(graph: &GraphContext, effects: &[(HostEffectKey, &dyn Fn())]) -> Vec<u32> {
        let mut order = Vec::new();
        loop {
            let mut due = Vec::new();
            graph.take_due_host_effects(&mut due).unwrap();
            if due.is_empty() {
                return order;
            }
            for tag in due {
                let (key, run) = effects[tag as usize];
                if graph.begin_effect(key).unwrap() {
                    run();
                    graph.end_effect(key).unwrap();
                    order.push(tag);
                }
            }
        }
    }

    #[test]
    fn host_effects_track_reads_and_requeue_on_their_own_writes() {
        let graph = GraphContext::new();
        let a = graph.signal().unwrap();
        let b = graph.signal().unwrap();
        let first = graph.host_effect(0).unwrap();
        let second = graph.host_effect(1).unwrap();
        let writes = Cell::new(0);
        let read_a = || graph.track(a).unwrap();
        // Writes what it reads, twice, then stops: each write makes it due again.
        let read_b_and_write = || {
            graph.track(b).unwrap();
            if writes.get() < 2 {
                writes.set(writes.get() + 1);
                graph.notify(b).unwrap();
            }
        };
        let effects: [(HostEffectKey, &dyn Fn()); 2] =
            [(first, &read_a), (second, &read_b_and_write)];
        // Both are due on creation, in creation order.
        assert_eq!(run_due(&graph, &effects), [0, 1, 1, 1]);
        assert!(run_due(&graph, &effects).is_empty());
        graph.notify(a).unwrap();
        assert_eq!(run_due(&graph, &effects), [0]);
        // A batch reports one run however many writes it holds.
        graph.begin_batch().unwrap();
        graph.notify(a).unwrap();
        graph.notify(a).unwrap();
        graph.end_batch().unwrap();
        assert_eq!(run_due(&graph, &effects), [0]);
        // A removed effect is no longer due, and cannot begin.
        graph.notify(a).unwrap();
        graph.remove_host_effect(first).unwrap();
        assert!(run_due(&graph, &effects).is_empty());
        assert!(!graph.begin_effect(first).unwrap());
        assert_eq!(graph.stats().unwrap().effects, 1);
        // An end with no open scope does nothing, and a foreign key is refused.
        graph.end_effect(second).unwrap();
        let other = GraphContext::new();
        assert!(other.begin_effect(second).is_err());
        graph.dispose();
        assert!(graph.host_effect(2).is_err());
    }

    #[test]
    fn writes_inside_a_host_scope_wait_for_the_outermost_end() {
        let graph = GraphContext::new();
        let s = graph.signal().unwrap();
        let outer = graph.host_effect(0).unwrap();
        let inner = graph.host_effect(1).unwrap();
        let mut due = Vec::new();
        graph.take_due_host_effects(&mut due).unwrap();
        assert_eq!(due, [0, 1]);
        assert!(graph.begin_effect(outer).unwrap());
        graph.track(s).unwrap();
        assert!(graph.begin_effect(inner).unwrap());
        graph.notify(s).unwrap();
        // The inner scope is abandoned when the outer one ends.
        graph.end_effect(outer).unwrap();
        due.clear();
        graph.take_due_host_effects(&mut due).unwrap();
        assert_eq!(
            due,
            [0],
            "the outer effect read s and is due after the write applies"
        );
        assert!(graph.begin_effect(inner).unwrap());
        graph.end_effect(inner).unwrap();
    }
}
