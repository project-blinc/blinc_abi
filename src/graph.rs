//! Owned Blinc dependency graph. Host values stay in the language adapter;
//! native signals/deriveds carry unit values and own dependency scheduling.
use blinc_core::reactive::{Derived, Effect, ReactiveGraph, Signal};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
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
        if ACTIVE.with(Cell::get).is_some() || self.draining.replace(true) {
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
}
