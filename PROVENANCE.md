# Provenance

- crates.io packages: `gpui-pre`, `gpui-pre-apple`, `gpui-pre-macos`, `gpui-pre-wgpu`, and `gpui-pre-windows` 0.3.4
- declared upstream repository: https://github.com/zed-industries/zed
- Zed snapshot recorded by the crates: `69164008341295ad481bb11c0334a712ca8c23e3`
- local deltas:
  - `patches/gpui-pre-macos-embedded-runloop.patch`
  - `crates/gpui-pre/src/scene.rs`, `crates/gpui-pre/src/bounds_tree.rs`
  - 2D paint transforms across `crates/gpui-pre`, `crates/gpui-pre-apple`, `crates/gpui-pre-wgpu`, and `crates/gpui-pre-windows`

The macOS patch touches only `src/platform.rs`. It adds an embedded macOS event-loop mode so GPUI can run inside a host process (Retend's Node/napi binding) instead of taking over the process with a blocking `CFRunLoopRun()`.

The original `gpui-pre` performance patch has two parts:

- `src/scene.rs` replaces per-primitive bounds-tree insertion during `Scene::replay` with a single reserved order range for the whole cached fragment. This keeps cached paint replay linear in the number of primitives. Scene unit tests cover overlap ordering, layer ordering, active-layer inheritance, and clipped fragment extents.
- `src/bounds_tree.rs` replaces the unbalanced "nested internal" overflow behavior with a balanced linear-split R-tree. The upstream snapshot builds a caterpillar of depth O(n / MAX_CHILDREN) under monotonic insertion (long lists, stacked rows), making ordering look quadratic per frame. Splitting full nodes into siblings keeps the tree at O(log n) depth. Tests cover balance under column insertion and brute-force ordering equivalence.

The transform patch is renderer-wide. Core GPUI assigns transformed primitives a compact `SpatialId` that references shared affine/clip state, while identity primitives stay on the original rendering path without spatial uploads or shader transform work. Hit-testing and input coordinates are mapped back into element-local space, and sparse spatial/clip state is remapped through scene replay. `gpui-pre-apple`, `gpui-pre-wgpu`, and `gpui-pre-windows` use separate transformed pipelines while preserving the upstream GPU record sizes and shaders for identity batches. This keeps transforms in the paint path rather than forcing layout changes or attaching a full matrix to every primitive.

Apple, WGPU, and Windows are included here because that renderer contract changed. They are sibling crates from the same `gpui-pre` 0.3.4 / Zed snapshot, not unrelated third-party dependencies, and must move in lockstep with the patched core crate.

The Windows implementation targets Direct3D 11 (`vs_4_1`/`ps_4_1` so 10.1-level hardware stays supported). Instance records keep their upstream strides with the CPU-only draw order replaced by the spatial id; spatial states and ancestor clips upload to `t2`/`t3` and transformed draws select `*_transformed` shader entry points. Paths are baked on the CPU into the screen-space intermediate texture instead of growing new path shaders.

To refresh against a newer snapshot:

1. Extract matching `gpui-pre`, `gpui-pre-apple`, `gpui-pre-macos`, `gpui-pre-wgpu`, and `gpui-pre-windows` crates from crates.io onto a clean branch.
2. Re-apply the embedded macOS run-loop patch and rebase the core/renderer transform and performance changes.
3. Keep all five Cargo package versions aligned with the snapshot being replaced.
4. Point `retend-gpui` at the new commit and bump every patched `gpui-pre*` pin together.
