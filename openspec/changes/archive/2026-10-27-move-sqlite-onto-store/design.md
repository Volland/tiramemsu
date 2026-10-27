## Decisions

### D-1. Store operations map to today's statements

Each `ReadStore`/`WriteStore` call on SQLite issues the statement the old engine issued for the same purpose (same SQL text, same indexes), so plans do not change. The oracle test of `add-store-trait` keeps running, now comparing the trait engine on SQLite with the trait engine on `MemStore`.

### D-2. Batch-friendly operations stay set-based

Where the SQL engine used one statement for many rows (cascade frontier `IN` lists, `load_rows`), the store trait gets a batch variant (`children_of(&[eid])`, `statements(&[eid])`) with a default per-row implementation, which SQLite overrides with the set-based statements.

### D-3. Regression gate

`crates/tiramemsu/benches/core.rs` and `sparql.rs` run before and after on the same machine; a median slowdown above 5 % on any benchmark blocks the change.

## Gate result (2026-10-27): not met

Phase A is built: `tm_core::sqlstore::SqlBackend` runs the trait engine on a SQLite file. Its evidence, in `crates/tm-store/tests/sqlite_backend.rs`:

- the oracle over 64 random histories and a fixed history, comparing it with the SQL engine (text index off, since SQLite's FTS5 lives outside the traits);
- a file written by the SQL engine and continued by both engines gives identical rows (`old_files_keep_working`).

The raw-row contract checks do not apply: SQLite's invariant triggers refuse statement rows without a transaction row.

The gate (`cargo run --release -p tiramemsu --example write_gate -- 400`, Apple M2 Max, medians of 400 interleaved runs):

| Operation | SQL engine | Trait engine on SQLite | Change |
|---|---|---|---|
| assert 1 | 111 µs | 110 µs | −1.2 % |
| assert 100 | 8 249 µs | 8 716 µs | +5.7 % |
| replace (cardinality one) | 115 µs | 196 µs | +69.9 % |
| annotated assert + retract | 235 µs | 449 µs | +91.1 % |
| supersede, 100 layers (dry run) | 16 652 µs | 2 600 µs | −84.4 % |

Three medians are more than 5 % slower, so by D-3 the facade keeps writing SQLite through the SQL engine and this change stays open. The slow cases issue one statement per row where the SQL engine uses one set-based statement (cardinality retraction, the cascade frontier); that is the batch work of D-2. The supersede case runs 6× faster on the trait engine, which points at a slow path in the SQL engine's supersede worth fixing on its own.

## Closing (2026-10-27)

Closed by decision after the gate: moving SQLite onto the trait engine buys no speed and costs 70–91 % on cardinality replacement and annotated retractions; its only gain would be one copy of the transaction semantics. SQLite keeps the SQL engine, and the two engines stay equal through the oracle, which now also runs the trait engine on SQLite (`SqlBackend`). Archived with `--skip-specs`, so `storage-trait` does not gain the requirements of this change. The supersede slow path of the SQL engine (6× slower than the trait engine's) is a separate fix.
