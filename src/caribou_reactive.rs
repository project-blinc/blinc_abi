//! Signals, computeds and host-run effects as Caribou plugin classes: the graph of `reactive`,
//! reached from any Caribou language.
//!
//! A computed holds a callback. It reads signals through the getters here, which record the read
//! as a dependency while it runs. Its result is the callback's return value; a style-value
//! computed hands its value back through `Computed.return_value` instead. A callback that raises
//! leaves the type's default.

use crate::caribou::StyleValue;
use crate::reactive::{
    AnyComputed, AnySignal, Slot, computed_value, computed_with, give, host_effect_begin,
    host_effect_end, host_effect_new, host_effect_release, host_effects_due, new_state, read,
    set_primitive, take,
};
use crate::types::Value;
use caribou_abi::host::{self, Kept, Text};
use caribou_abi::{BufferMut, Value as Host};
use std::sync::Arc;

/// A signal of one type: a number, a boolean, a text that may be absent, or a style value.
pub struct Signal(pub(crate) AnySignal);

/// A computed of one type, following the signals its callback reads.
pub struct Computed(pub(crate) AnyComputed);

fn text_of(s: Option<String>) -> Text {
    host::text(s.as_deref().unwrap_or(""))
}

fn number(v: Host) -> Option<f64> {
    v.as_number().or_else(|| v.as_int().map(f64::from))
}

/// What `callback` returns, converted by `convert`, or the default when it raises or answers another type.
fn called<T: Slot>(callback: &Kept, convert: fn(Host) -> Option<T>) -> T {
    match host::call(callback.get(), &[]) {
        Ok(v) => convert(v).unwrap_or_default(),
        Err(_) => T::default(),
    }
}

fn computed_of_callback<T: Slot>(
    callback: Host,
    convert: fn(Host) -> Option<T>,
) -> blinc_core::reactive::Computed<T> {
    let callback = Arc::new(Kept::new(callback));
    computed_with(move || called(&callback, convert))
}

fn as_i32(v: Host) -> Option<i32> {
    v.as_int().or_else(|| v.as_number().map(|n| n as i32))
}

fn as_f32(v: Host) -> Option<f32> {
    number(v).map(|n| n as f32)
}

fn as_text(v: Host) -> Option<Option<String>> {
    if v.is_null() {
        return Some(None);
    }
    Text::of(v).map(|t| Some(t.as_str().to_string()))
}

impl Signal {
    pub extern "C" fn int(initial: i32) -> Box<Signal> {
        Box::new(Signal(AnySignal::I32(new_state(initial))))
    }

    pub extern "C" fn float(initial: f32) -> Box<Signal> {
        Box::new(Signal(AnySignal::F32(new_state(initial))))
    }

    pub extern "C" fn double(initial: f64) -> Box<Signal> {
        Box::new(Signal(AnySignal::F64(new_state(initial))))
    }

    pub extern "C" fn bool(initial: bool) -> Box<Signal> {
        Box::new(Signal(AnySignal::Bool(new_state(initial))))
    }

    pub extern "C" fn text(initial: Text) -> Box<Signal> {
        Box::new(Signal(AnySignal::Str(new_state(Some(
            initial.as_str().to_string(),
        )))))
    }

    /// A text signal holding no text.
    pub extern "C" fn none() -> Box<Signal> {
        Box::new(Signal(AnySignal::Str(new_state(None))))
    }

    pub extern "C" fn value(initial: &StyleValue) -> Box<Signal> {
        Box::new(Signal(AnySignal::Value(new_state(initial.inner().clone()))))
    }

    pub extern "C" fn get_int(this: &Signal) -> i32 {
        i32::state(&this.0).map(read).unwrap_or_default()
    }

    pub extern "C" fn get_float(this: &Signal) -> f32 {
        <f32 as Slot>::state(&this.0).map(read).unwrap_or_default()
    }

    pub extern "C" fn get_double(this: &Signal) -> f64 {
        f64::state(&this.0).map(read).unwrap_or_default()
    }

    pub extern "C" fn get_bool(this: &Signal) -> bool {
        bool::state(&this.0).map(read).unwrap_or_default()
    }

    /// The text, empty when there is none; `has_text` tells them apart.
    pub extern "C" fn get_text(this: &Signal) -> Text {
        text_of(<Option<String>>::state(&this.0).and_then(read))
    }

    /// Whether a text signal holds text.
    pub extern "C" fn has_text(this: &Signal) -> bool {
        <Option<String>>::state(&this.0)
            .and_then(read)
            .is_some()
    }

    /// How many times a text signal changed, read without recording a dependency; -1 for another type.
    pub extern "C" fn text_version(this: &Signal) -> i32 {
        <Option<String>>::state(&this.0)
            .and_then(|s| s.signal().version())
            .map_or(-1, |v| v as i32)
    }

    pub extern "C" fn set_int(this: &Signal, v: i32) {
        if let Some(s) = i32::state(&this.0) {
            set_primitive(s, v);
        }
    }

    pub extern "C" fn set_float(this: &Signal, v: f32) {
        if let Some(s) = <f32 as Slot>::state(&this.0) {
            set_primitive(s, v);
        }
    }

    pub extern "C" fn set_double(this: &Signal, v: f64) {
        if let Some(s) = f64::state(&this.0) {
            set_primitive(s, v);
        }
    }

    pub extern "C" fn set_bool(this: &Signal, v: bool) {
        if let Some(s) = bool::state(&this.0) {
            set_primitive(s, v);
        }
    }

    pub extern "C" fn set_text(this: &Signal, v: Text) {
        if let Some(s) = <Option<String>>::state(&this.0) {
            s.set(Some(v.as_str().to_string()));
        }
    }

    pub extern "C" fn set_none(this: &Signal) {
        if let Some(s) = <Option<String>>::state(&this.0) {
            s.set(None);
        }
    }

    pub extern "C" fn set_value(this: &Signal, v: &StyleValue) {
        if let Some(s) = <Value>::state(&this.0) {
            s.set(v.inner().clone());
        }
    }

    /// Reads the signal for its side effect alone: inside a computation, recording it as a dependency.
    pub extern "C" fn touch(this: &Signal) {
        match &this.0 {
            AnySignal::I32(s) => drop(read(s)),
            AnySignal::F32(s) => drop(read(s)),
            AnySignal::F64(s) => drop(read(s)),
            AnySignal::Bool(s) => drop(read(s)),
            AnySignal::Str(s) => drop(read(s)),
            AnySignal::Value(s) => drop(read(s)),
        }
    }
}

impl Computed {
    pub extern "C" fn int(callback: Host) -> Box<Computed> {
        Box::new(Computed(AnyComputed::I32(computed_of_callback(
            callback, as_i32,
        ))))
    }

    pub extern "C" fn float(callback: Host) -> Box<Computed> {
        Box::new(Computed(AnyComputed::F32(computed_of_callback(
            callback, as_f32,
        ))))
    }

    pub extern "C" fn double(callback: Host) -> Box<Computed> {
        Box::new(Computed(AnyComputed::F64(computed_of_callback(
            callback, number,
        ))))
    }

    pub extern "C" fn bool(callback: Host) -> Box<Computed> {
        Box::new(Computed(AnyComputed::Bool(computed_of_callback(
            callback,
            |v| v.as_bool(),
        ))))
    }

    /// A text computed: the callback returns a text, or null for none.
    pub extern "C" fn text(callback: Host) -> Box<Computed> {
        Box::new(Computed(AnyComputed::Str(computed_of_callback(
            callback, as_text,
        ))))
    }

    /// A style-value computed: the callback calls `return_value` with its result.
    pub extern "C" fn value(callback: Host) -> Box<Computed> {
        let callback = Arc::new(Kept::new(callback));
        let c = computed_with(move || {
            take::<Value>();
            let _ = host::call(callback.get(), &[]);
            take::<Value>().unwrap_or_default()
        });
        Box::new(Computed(AnyComputed::Value(c)))
    }

    /// Hands back the style value the running computed callback produced.
    pub extern "C" fn return_value(v: &StyleValue) {
        give(v.inner().clone());
    }

    pub extern "C" fn get_int(this: &Computed) -> i32 {
        i32::computed(&this.0).map(computed_value).unwrap_or_default()
    }

    pub extern "C" fn get_float(this: &Computed) -> f32 {
        <f32 as Slot>::computed(&this.0)
            .map(computed_value)
            .unwrap_or_default()
    }

    pub extern "C" fn get_double(this: &Computed) -> f64 {
        f64::computed(&this.0)
            .map(computed_value)
            .unwrap_or_default()
    }

    pub extern "C" fn get_bool(this: &Computed) -> bool {
        bool::computed(&this.0)
            .map(computed_value)
            .unwrap_or_default()
    }

    /// The text, empty when there is none.
    pub extern "C" fn get_text(this: &Computed) -> Text {
        text_of(<Option<String>>::computed(&this.0).and_then(computed_value))
    }

    /// Whether a text computed holds text.
    pub extern "C" fn has_text(this: &Computed) -> bool {
        <Option<String>>::computed(&this.0)
            .and_then(computed_value)
            .is_some()
    }

    /// Brings the computed up to date, running its callback if a dependency changed.
    pub extern "C" fn touch(this: &Computed) {
        match &this.0 {
            AnyComputed::I32(c) => drop(c.try_get()),
            AnyComputed::F32(c) => drop(c.try_get()),
            AnyComputed::F64(c) => drop(c.try_get()),
            AnyComputed::Bool(c) => drop(c.try_get()),
            AnyComputed::Str(c) => drop(c.try_get()),
            AnyComputed::Value(c) => drop(c.try_get()),
        }
    }

    /// Releases the computed before it is collected, as its owner does when disposed; it takes effect at the next flush.
    pub extern "C" fn release(this: &Computed) {
        this.0.release();
    }
}

/// Effects whose body is the caller's: it takes the due ones at a flush and runs each between
/// `begin` and `end`, which track what the body reads.
pub struct Effects;

impl Effects {
    /// A new effect's slot. It starts due; its first run is the caller's. Not for use while a
    /// computed's callback runs.
    pub extern "C" fn create() -> i32 {
        host_effect_new()
    }

    /// Writes the slots of due effects into `out` as i32s, in the order they became due; how many.
    pub extern "C" fn due(out: BufferMut) -> i32 {
        let room = out.len() / 4;
        let at = out.as_mut_ptr() as *mut i32;
        if room == 0 || at.is_null() {
            return 0;
        }
        let mut slots = vec![0i32; room];
        let n = host_effects_due(&mut slots) as usize;
        for (i, s) in slots[..n].iter().enumerate() {
            unsafe { at.add(i).write_unaligned(*s) };
        }
        n as i32
    }

    /// Opens the effect in `slot` for its body; false for a slot with no effect.
    pub extern "C" fn begin(slot: i32) -> bool {
        host_effect_begin(slot)
    }

    /// Closes the effect in `slot` and applies the signal writes its body made; called on every way out of the body.
    pub extern "C" fn end(slot: i32) {
        host_effect_end(slot);
    }

    /// Releases the effect in `slot`; the next flush removes it, and the slot may then name another.
    pub extern "C" fn release(slot: i32) {
        host_effect_release(slot);
    }

    /// Whether any state changed since the last `clear_dirty`.
    pub extern "C" fn is_dirty() -> bool {
        crate::reactive::is_dirty()
    }

    pub extern "C" fn clear_dirty() {
        crate::reactive::clear_dirty();
    }
}
