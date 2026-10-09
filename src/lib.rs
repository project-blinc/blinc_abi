// Every `unsafe extern "C"` export here is called only by the HashLink
// runtime, and its contract is the matching Haxe extern declaration.
#![allow(clippy::missing_safety_doc)]

#[cfg(all(feature = "hashlink", target_os = "macos"))]
mod alloc;
#[cfg(all(feature = "hashlink", target_os = "macos"))]
#[global_allocator]
static ALLOCATOR: alloc::BigBlocksMapped = alloc::BigBlocksMapped;

pub mod context;
pub mod graph;
#[cfg(feature = "scene")]
pub mod scene;
pub mod tree;

#[cfg(feature = "scene")]
pub mod bitmap;
#[cfg(feature = "scene")]
pub mod display_list;
mod grid;
#[cfg(feature = "scene")]
pub mod hit;
#[cfg(feature = "hashlink")]
mod hl;
pub mod layout_props;
#[cfg(feature = "hashlink")]
pub mod layout_router;
#[cfg(feature = "hashlink")]
pub mod node;
#[cfg(feature = "hashlink")]
pub mod reactive;
#[cfg(feature = "scene")]
pub mod svg;
#[cfg(feature = "scene")]
pub mod text;
pub mod types;

/// Tells Ash this library never stores a GC pointer into a GC object itself:
/// it keeps Haxe objects only through roots (`hl::Rooted`) and writes numbers
/// into byte buffers. Ash's card-marking write barrier is then safe with it
/// loaded.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub static ash_hdll_barrier_aware: u8 = 1;
