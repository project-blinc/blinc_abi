// Every `unsafe extern "C"` export here is called only by the HashLink
// runtime, and its contract is the matching Haxe extern declaration.
#![allow(clippy::missing_safety_doc)]

#[cfg(all(feature = "hashlink", target_os = "macos"))]
mod alloc;
#[cfg(all(feature = "hashlink", target_os = "macos"))]
#[global_allocator]
static ALLOCATOR: alloc::BigBlocksMapped = alloc::BigBlocksMapped;

pub mod context;
pub mod css;
#[cfg(any(feature = "hashlink", feature = "caribou"))]
mod css_hl;
pub mod graph;
#[cfg(test)]
mod row_grid;
#[cfg(feature = "scene")]
pub mod scene;
pub mod tree;

#[cfg(feature = "scene")]
pub mod bitmap;
#[cfg(feature = "caribou")]
mod caribou;
#[cfg(feature = "scene")]
pub mod display_list;
#[cfg(feature = "scene")]
pub mod font;
mod grid;
#[cfg(feature = "scene")]
pub mod hit;
#[cfg(feature = "hashlink")]
mod hl;
#[cfg(feature = "scene")]
pub mod inline;
pub mod layout_props;
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub mod layout_router;
pub mod layout_tree;
#[cfg(feature = "hashlink")]
pub mod node;
#[cfg(feature = "scene")]
mod notch;
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) mod ops;
#[cfg(feature = "hashlink")]
pub mod reactive;
#[cfg(any(feature = "hashlink", feature = "caribou"))]
pub(crate) mod shared;
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
