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

The `scene` feature adds owned scene properties, measured text, display-list
encoding and hit testing without HashLink. Each `SceneEncoder` retains reusable
buffers and its own glyph atlases, with revisioned incremental uploads. It reuses
the existing paint and hit-test logic. Text/image helpers are also available;
Node resource bindings and GPU integration remain in progress.

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
