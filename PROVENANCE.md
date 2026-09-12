# Provenance

- crates.io packages: `gpui-pre-macos` 0.3.4 and `gpui-pre` 0.3.4
- declared upstream repository: https://github.com/zed-industries/zed
- Zed snapshot recorded by the crates: `69164008341295ad481bb11c0334a712ca8c23e3`
- local deltas:
  - `patches/gpui-pre-macos-embedded-runloop.patch`
  - `crates/gpui-pre/src/scene.rs`, `crates/gpui-pre/src/bounds_tree.rs`

The macOS patch touches only `src/platform.rs`. It adds an embedded macOS event-loop mode so GPUI can run inside a host process (Retend's Node/napi binding) instead of taking over the process with a blocking `CFRunLoopRun()`.

The `gpui-pre` patch has two parts:

- `src/scene.rs` replaces per-primitive bounds-tree insertion during `Scene::replay` with a single reserved order range for the whole cached fragment. This keeps cached paint replay linear in the number of primitives. Scene unit tests cover overlap ordering, layer ordering, active-layer inheritance, and clipped fragment extents.
- `src/bounds_tree.rs` replaces the unbalanced "nested internal" overflow behavior with a balanced linear-split R-tree. The upstream snapshot build a caterpillar of depth O(n / MAX_CHILDREN) under monotonic insertion (long lists, stacked rows), making ordering lookups quadratic per frame. Splitting full nodes into siblings keeps the tree at O(log n) depth. Tests cover balance under column insertion and brute-force ordering equivalence.

To refresh against a newer snapshot:

1. Extract the new `gpui-pre-macos` crate from crates.io onto a clean branch.
2. Re-apply `patches/gpui-pre-macos-embedded-runloop.patch`, or rebase the current `src/platform.rs` onto the new file.
3. Keep the Cargo package version aligned with the snapshot being replaced.
4. Point `retend-gpui` at the new commit and bump every `gpui-pre*` pin together.
