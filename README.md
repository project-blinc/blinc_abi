# blinc_abi

Native ABI for Blinc layout, reactivity, text, images and rendering primitives.
Language SDKs supply their own components and renderer orchestration.

## Extraction status

The default `hashlink` feature builds the existing HashLink/Ash exports.
`default-features = false` exposes owned Rust layout and reactive contexts to language
adapters without linking HashLink or its GC roots. Node uses this context through
[blinc_ts](https://github.com/project-blinc/blinc_ts).

The layout context validates node ownership and generations, rejects cycles,
supports reordering/reparenting, and writes absolute bounds into caller-owned
buffers. Disposing it invalidates all its nodes. The reactive context provides
isolated signals, computed dependencies, batched effects and deferred mutation
while callbacks run. Host adapters retain values and callback references.

`LayoutContext::apply` accepts the shared `context::TEXT_ALIGN` router id (40):
`PropValue::Enum` selects left (0), centre (1) or right (2), and `Unset` clears
the override. The staged batch preserves the node's layout style and other
paint fields, and a rejected write leaves the entire batch unchanged.

`LayoutContext::set_grid_rows(group, rows)` opts direct rows into a shared grid
for table row spans. Adapters assign columns and spans; native layout measures
the shared tracks and keeps authored row/cell ancestry for styles and events.
Ordinary row backgrounds paint below spanning cells, with row transforms,
clipping and stacking contexts preserved. An empty row list restores ordinary
layout. HashLink exposes the same operation as `blinc_tree_set_grid_rows`, taking
consecutive 64-bit row IDs and returning false for invalid ownership or rows.

The `scene` feature adds owned scene properties, measured text, display-list
encoding and hit testing without HashLink. Each `SceneEncoder` retains reusable
buffers and its own glyph atlases, with revisioned incremental uploads. It reuses
the existing paint and hit-test logic. Text/image helpers are also available;
Node resource bindings and GPU integration remain in progress.

Owned `font::Font` resources decode TTF, OpenType and WOFF2 bytes, or retain an
installed face through `Font::local`. `family(weight)` registers an immutable
private alias in the same Blinc registry used by measurement and scene encoding.
Dropping the resource unregisters its aliases; faces already retained by a
renderer remain valid. Adapters own URL loading and stylesheet lifetime.

CSS sheets collect typed `@font-face` descriptors and source locations, including
imports and media conditions. `Cascade::font_faces` exposes active faces, and
`css::font_face::match_face` implements CSS weight/style selection. Compiled CSS
uses version 3 when faces are present; font-free sheets keep version 2, and both
versions decode. WOFF2 decoding currently excludes transformed `hmtx` tables.
Original WOFF, unicode ranges, stretch and font-display timing are follow-up work.

The ABI generates display lists; GPU execution is supplied by a consuming
renderer through xgpu. Window integration is supplied through xwindow.

## Check the extraction

```sh
cargo check --locked
cargo test --locked --no-default-features --lib
cargo test --locked --no-default-features --features scene --lib
cargo build --release --locked
```

The existing HashLink adapter produces a dynamic or static library. Windows
linking also needs the HashLink import library, selected with `HL_LIB_DIR`.

## SDKs

- [blinc_ts](https://github.com/project-blinc/blinc_ts): TypeScript SDK and Node integration.
- [ashui](https://github.com/rayzor-blade/ashui): existing Haxe integration.

See [adapter separation and compatibility checks](docs/adapters.md) and
[contributing](docs/contributing.md).

Apache-2.0. Source provenance is recorded in [NOTICE](NOTICE).
