# gpui-pre

Patched snapshots of the `gpui-pre-*` crates used by [retend-gpui](https://github.com/resuite/retend).

This repository is not Zed and it is not Retend. It exists so Retend can depend on a pinned GPUI macOS backend without vendoring that backend inside the Retend tree.

## Current crate

- `crates/gpui-pre-macos` — `gpui-pre-macos` 0.3.4 plus an embedded NSApplication / CFRunLoop pump in `src/platform.rs`

Keep the Cargo package name `gpui-pre-macos`. Downstream `[patch.crates-io]` entries rely on that name.

## Depend on it

Pin a commit. Do not float `main`.

```toml
[patch.crates-io]
gpui-pre-macos = { git = "https://github.com/resuite/gpui-pre", rev = "<full-40-char-sha>" }
```

Local checkout:

```toml
[patch.crates-io]
gpui-pre-macos = { path = "../gpui-pre/crates/gpui-pre-macos" }
```

The rest of the `gpui-pre` family (`gpui-pre`, `gpui-pre-platform`, …) stays on crates.io at `=0.3.4` until this repo snapshots those crates too.

## License

Apache-2.0. GPUI source is from Zed Industries; see `crates/gpui-pre-macos/LICENSE-APACHE` and `PROVENANCE.md`.
