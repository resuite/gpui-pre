# gpui-pre

Patched snapshots of the `gpui-pre-*` crates used by [retend-gpui](https://github.com/resuite/retend).

This repository is not Zed and it is not Retend. It exists so Retend can depend on a pinned GPUI renderer snapshot without keeping those patches inside the Retend tree.

## Current crates

- `crates/gpui-pre` — `gpui-pre` 0.3.4 plus Retend's scene replay, bounds-tree, and sparse 2D spatial-transform support
- `crates/gpui-pre-apple` — `gpui-pre-apple` 0.3.4 plus Metal support for sparse transformed primitives and clips
- `crates/gpui-pre-macos` — `gpui-pre-macos` 0.3.4 plus an embedded NSApplication / CFRunLoop pump in `src/platform.rs`
- `crates/gpui-pre-wgpu` — `gpui-pre-wgpu` 0.3.4 plus WGPU support for sparse transformed primitives and clips
- `crates/gpui-pre-windows` — `gpui-pre-windows` 0.3.4 plus Direct3D 11 support for sparse transformed primitives and clips

These five crates form one patched renderer snapshot and should be pinned to the same repository revision. Keep the Cargo package names unchanged; downstream `[patch.crates-io]` entries rely on them.

## Depend on it

Pin a commit. Do not float `main`.

```toml
[patch.crates-io]
gpui-pre = { git = "https://github.com/resuite/gpui-pre", rev = "<full-40-char-sha>" }
gpui-pre-apple = { git = "https://github.com/resuite/gpui-pre", rev = "<full-40-char-sha>" }
gpui-pre-macos = { git = "https://github.com/resuite/gpui-pre", rev = "<full-40-char-sha>" }
gpui-pre-wgpu = { git = "https://github.com/resuite/gpui-pre", rev = "<full-40-char-sha>" }
gpui-pre-windows = { git = "https://github.com/resuite/gpui-pre", rev = "<full-40-char-sha>" }
```

Local checkout:

```toml
[patch.crates-io]
gpui-pre = { path = "../gpui-pre/crates/gpui-pre" }
gpui-pre-apple = { path = "../gpui-pre/crates/gpui-pre-apple" }
gpui-pre-macos = { path = "../gpui-pre/crates/gpui-pre-macos" }
gpui-pre-wgpu = { path = "../gpui-pre/crates/gpui-pre-wgpu" }
gpui-pre-windows = { path = "../gpui-pre/crates/gpui-pre-windows" }
```

Cargo patches packages individually, so consumers need one entry per modified package even though all four come from the same pinned repository revision. The rest of the `gpui-pre` family (`gpui-pre-platform`, …) stays on crates.io at `=0.3.4` until this repository needs to patch those crates too.

## License

Apache-2.0. GPUI source is from Zed Industries; see the crate license files and `PROVENANCE.md`.
