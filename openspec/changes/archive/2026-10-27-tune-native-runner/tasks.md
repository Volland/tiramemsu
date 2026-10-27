## Status

Implemented 2026-10-27.

## 1. Storage reads

- [x] 1.1 Fixed-width redb values as `&[u8; N]` (zero-copy, same type name).
- [x] 1.2 Key prefixes and range ends on the stack (`keys::Prefix`); the pattern check is skipped when the prefix covers every bound position.

## 2. Join engine

- [x] 2.1 Flat join rows (`bgp::Rows`); no allocation per extended row.
- [x] 2.2 Hash joins over build rows sorted by an Fx hash of the shared slots; `bind` rejects colliding rows.
- [x] 2.3 Generic join without per-key allocations; Fx hashing for tries, grouping and merges.
- [x] 2.4 `merge` with the unit relation returns the other side.
- [x] 2.5 Join order tried from every first pattern — `@tg: implements:: [[openspec:native-query-execution#Cost-based join order]]`.
- [x] 2.6 Constant equalities bind their variable before the join — `@tg: implements:: [[openspec:native-query-execution#Constant equalities seek]]`; a lone pattern under filters goes through `join_with`.

## 3. Verification

- [x] 3.1 Tests for the new scenarios in `crates/tiramemsu/tests/native_runner.rs`.
- [x] 3.2 Workspace tests, the differential corpus and the property test pass.
- [x] 3.3 Benchmarks at 10⁵ and 10⁶ statements with `tm-bench --example probe`, every answer compared with SQLite before timing.
