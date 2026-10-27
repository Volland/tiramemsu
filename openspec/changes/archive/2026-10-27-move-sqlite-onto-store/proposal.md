# Move the SQLite engine onto the storage trait

Target release: **0.9**. Last of six changes (phase 2 of D41 in `lat.md/overview.md#Decision Record`). Depends on `add-store-trait`; benefits from the oracle evidence of `add-redb-backend` and `add-native-runner`.

## Why

After `add-store-trait`, the transaction semantics exist twice: the original SQL engine in `tm-core` and the trait engine in `tm-store`. The oracle test keeps them equal, but every fix still has to be made in both. Once the trait engine has proven itself on redb, SQLite should run on it too, so the semantics live once.

## What Changes

- **`tm-rusqlite` (and `tm-wasm`) implement `ReadStore`/`WriteStore`** with their existing tables, indexes and prepared statements; scans keep the `+?N` fixed-plan shapes; the cascade keeps its set-based recursive query as a store operation.
- **The facade runs SQLite writes on `tm_store::Engine`.** `tm_core::Store`/`Tx` become thin re-exports of the trait engine for compatibility; their SQL bodies are removed.
- **Regression gate:** the existing core and SPARQL benchmarks must not be more than 5 % slower on SQLite, or the change does not land.
- **`OpenOptions::runner = Native`** becomes available on SQLite files (queries through `tm-native` over the SQLite store), for comparison; `tm-exec` stays the default SQLite runner.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `storage-trait`: adds the SQLite implementation and the regression gate (ADDED requirements).

## Impact

- `crates/tm-core/src/engine/` (removed after the port), `crates/tm-rusqlite`, `crates/tm-wasm`, `crates/tiramemsu/src/db.rs`.
- **Requirements added:** [[openspec:storage-trait#SQLite implements the storage trait]], [[openspec:storage-trait#No regression on SQLite]].

## Non-goals

- No change to the SQLite file format or to SQL query routing.
