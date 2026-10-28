# Depend on eyeron from crates.io

Source: decision N2 of `add-n3-reasoning` (design interview of 2026-10-28).

## Why

`tm-n3` depends on eyeron through a git pin because eyeron is not on crates.io. Cargo refuses to publish a crate with a git dependency, optional or not, so while the pin stays, `tm-n3` cannot be published and the facade cannot be published with its `n3` feature. eyeron's manifest also makes `ureq` (an HTTP client used only by its binaries) a dependency of every user of the library, and its `time:localTime` reads the system clock, which `tm-n3` currently works around by rewriting rules.

## What Changes

- **Upstream (eyeron, a pull request to be proposed to its maintainer):** a default-on `cli` feature that gates `ureq` and the binaries; `ReasonerOptions::now: Option<i64>` used by `time:localTime`; publishing the crate to crates.io. Wishlist, not required: all proofs per conclusion; a cancellation callback polled during reasoning.
- **Here, once published:** the workspace depends on `eyeron = { version = "…", default-features = false }`; `tm-n3` drops `publish = false` and the `time:localTime` rewrite in favor of `ReasonerOptions::now`; the release checklist publishes `tm-n3` before `tiramemsu`.

## Capabilities

### New Capabilities

- `eyeron-dependency`: how the reasoner dependency is sourced and what it may pull in.

## Impact

- `Cargo.toml` (workspace dependency), `crates/tm-n3/Cargo.toml`, `crates/tm-n3/src` (clock), release notes.

## Non-goals

- Vendoring or forking eyeron.
