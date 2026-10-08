# Runtime adapters

The extraction starts with the existing HashLink ABI so its behavior can be
verified before changing ownership or callback semantics.

## Shared engine

Keep layout trees, node properties, reactive dependency tracking, text/image
resources, display lists and hit testing in the shared engine. Export opaque
handles and explicitly described buffer layouts. Negotiate ABI versions before
independently releasing SDKs.

The compatibility adapter retains its process-global tree and reactive graph.
`context::LayoutContext` provides a separate, owned layout tree for other hosts.
Its opaque nodes include a context identity and generation; edits validate
ownership, duplicate children and cycles before changing the tree. Each host
serializes access to its context. The Node adapter owns it on the JavaScript
thread and disposes it with the mounted UI scope. Bounds are copied into a
caller-owned reusable buffer, so consumers retain no borrowed native storage.

## Host-specific code

Separate `hl.rs`, GC handle allocation, rooted foreign values, string conversion,
blocking notifications, primitive registration and callback invocation from the
shared operations. Existing `hl_blinc_*` exports remain the compatibility adapter.

The shared engine should receive host hooks for opaque foreign values and
callbacks. A HashLink adapter uses HL roots and `hl_dyn_call`; a Node adapter
uses N-API references and callbacks on the JavaScript thread. Preserve dependency
tracking across callbacks and explicitly handle callback errors.

Dropping a language wrapper and disposing a reactive scope are separate actions.
Match the current graph disposal rules, including signals/computeds bound to
live nodes. Do not free borrowed buffers or callback references while native
operations still use them.

## Rendering boundary

Display lists encode primitives in paint order, currently 112 floats per record
with polygon point payloads following them. Keep the producer and shader schema
aligned. Node adapters should lend stable views or copy at a documented ownership
boundary; SDK consumers must never retain a borrowed view past its lifetime.

Visual offsets and drawn sizes affect both painting and hit testing. Preserve
that contract for layout animation.

## Validation gates

1. Compile the extraction using its copied lockfile.
2. Compare exported HashLink symbols and rendered output with the existing ABI.
3. Introduce host-neutral operations behind the existing adapter.
4. Run Haxe regression and snapshot checks against the separated adapter.
5. Add Node lifecycle, callback, buffer and disposal tests.

Owned layout and reactive contexts are available without the default `hashlink`
feature. Core tests cover ownership, tree edits, batches and effect disposal;
Node integration tests cover callback identity/errors, dynamic dependencies,
nested effect cleanup and HMR scopes. The original adapter still compiles with
default features. The `scene` feature now builds the shared paint walk, hit testing
and text/image helpers independently of HashLink; owned scene APIs, Node resource
bindings and compatibility snapshots remain the next extraction milestone.
