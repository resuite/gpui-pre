# Provenance

- crates.io packages: `gpui-pre-macos` 0.3.4 and `gpui-pre` 0.3.4
- declared upstream repository: https://github.com/zed-industries/zed
- Zed snapshot recorded by the crates: `69164008341295ad481bb11c0334a712ca8c23e3`
- local deltas:
  - `patches/gpui-pre-macos-embedded-runloop.patch`

`crates/gpui-pre` is an unmodified snapshot of the published `gpui-pre` 0.3.4 crate.

The macOS patch touches only `src/platform.rs`. It adds an embedded macOS event-loop mode so GPUI can run inside a host process (Retend's Node/napi binding) instead of taking over the process with a blocking `CFRunLoopRun()`.

To refresh against a newer snapshot:

1. Extract the new `gpui-pre-macos` crate from crates.io onto a clean branch.
2. Re-apply `patches/gpui-pre-macos-embedded-runloop.patch`, or rebase the current `src/platform.rs` onto the new file.
3. Keep the Cargo package version aligned with the snapshot being replaced.
4. Point `retend-gpui` at the new commit and bump every `gpui-pre*` pin together.
