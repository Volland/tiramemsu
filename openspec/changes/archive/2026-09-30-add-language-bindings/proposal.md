## Why

Tiramemsu is a Rust library with no way in for the two ecosystems agents are written in. An agent framework in Python or TypeScript cannot use bitemporal, layered memory without writing Rust. The sibling project oxilite ships Node.js and Python packages built the same way (a small native crate over a JSON bridge, a typed wrapper on top), and that layout is proven, so this change reuses it.

The facade already anticipates this (`Patch::from_fields` takes named fields "as bindings receive it", and `CypherValue::to_json` exists), and `lat.md/api.md` lists a Python and a Node binding as planned. This change builds them and prepares both for publishing.

## What Changes

- **JSON bridge** (`bindings/json`, crate `tiramemsu-json`): one `Database` object that takes an operation name and a JSON object and returns JSON. It carries every conversion: terms, views on both clocks, transactions as lists of op objects with named references, speculation, and errors with codes. Each native crate is a constructor and one `call` method.
- **Node.js** (`bindings/node`, crate `tiramemsu-node`, npm `@tiramemsu/node`): a napi-rs addon and a typed TypeScript wrapper (`Database`, `View`, `Tx`), tested with vitest.
- **Python** (`bindings/python`, crate `tiramemsu-python`, PyPI `tiramemsu`): a PyO3 module on the stable ABI and a typed Python wrapper, with `Database.transact` as a context manager, tested with pytest and `mypy --strict`.
- **Release**: a `python-wheels` workflow (six wheels and an sdist, PyPI trusted publishing), an `npm-publish` workflow (one prebuilt addon per platform, npm provenance), CI jobs that build and test both packages on every push, and publishing guides.
- The workspace gains three members. The three binding crates do not inherit the workspace's `unsafe_code = "forbid"`, because the napi and pyo3 macros expand to `unsafe`. The facade and the core crates are unchanged.

Non-goals: WASM and MCP bindings (still planned in `lat.md/api.md`), an async API, streaming results, Python `datetime` timezone database handling beyond RFC 3339, and crypto-shredding.

## Capabilities

### New Capabilities

- `json-bridge`: the shared JSON call surface: operations, views, terms, transaction ops, errors.
- `node-binding`: the `@tiramemsu/node` package.
- `python-binding`: the `tiramemsu` package.
- `binding-release`: the workflows and guides that build and publish both packages.

### Modified Capabilities

None. No requirement of an existing capability changes.

## Impact

- New directories `bindings/json`, `bindings/node`, `bindings/python`; new workflows `.github/workflows/python-wheels.yml` and `npm-publish.yml`; new guides `docs/python-publishing.md` and `docs/node-publishing.md`; new job entries in `.github/workflows/ci.yml`.
- Root `Cargo.toml` members, `.gitignore`, `lat.md/bindings.md`, `lat.md/api.md`.
- `cargo test --workspace` and `cargo clippy --workspace` now also compile pyo3 and napi; they need a Python interpreter and Node headers only at link time of the packaged artifacts, not for `cargo check`.
