# Tune the native runner on redb

## Why

After 0.8.0 the redb engine lost to SQLite on probe-heavy plans: WatDiv C1 2.3–3.9×, S3 2.1–2.5×, the Cypher 2-hop 2.5–3.1× and, at 10⁶ statements, the snowflake F1 4.4×. Profiles (`sample` on macOS, `tm-bench` example `probe`) traced the losses to four causes in our own code, not to redb's B-tree:

- **Byte-by-byte value decoding.** Tables typed `[u8; N]` decode through redb's generic array impl, one `u8::from_bytes` per byte. It was the largest single cost of a scan.
- **Allocations per probe and per row.** Each probe built its key prefix in two vectors and its range end in a third; each output row was a new `Vec<i64>`; hash joins keyed on `Vec<i64>` with SipHash; `join_with` copied the whole BGP result once more by merging it into the unit relation.
- **Join order.** The greedy order always started from the smallest pattern. On F1 that is `?s locatedIn ?c` (1 052 rows), which grew 315 656 intermediate rows before the selective `?u livesIn city1` cut them.
- **Filters not reaching a lone pattern.** An `EXISTS` body or a Cypher property map is a pattern under filters; it was scanned in full and filtered afterwards, so `{name: 'User 7'}` read every `name` statement.

A larger redb cache was ruled out: 256 MiB and 2 GiB gave the same times at 10⁶ statements.

## What Changes

- `tm-redb` reads fixed-width values as `&[u8; N]` (the same redb type name, so files are unchanged), builds key prefixes on the stack and skips the pattern check when the prefix covers every bound position.
- `tm-native` keeps join rows flat in one buffer, sorts hash-join build rows by an Fx hash of the shared slots, avoids per-key allocations in the generic join and uses Fx hashing for tries and grouping; `merge` returns the other side unchanged when one side is the unit relation.
- **Join order** tries every pattern as the first access path and keeps the order with the fewest estimated intermediate rows (MODIFIED requirement *Cost-based join order*).
- **Constant equalities seek:** a conjunct `?v = k` whose constant the evaluator compares by identity binds `?v` before the join (ADDED requirement), and a lone pattern under filters is matched through the join so that its conjuncts reach it.

No file format, API or answer changes. `rustc-hash` (pure Rust) becomes a dependency of `tm-native`.

## Impact

- Specs: `native-query-execution`.
- Code: `crates/tm-redb/src/{keys,rows,store,tables}.rs`, `crates/tm-native/src/{bgp,exec}.rs`.
- Tests: three new integration tests in `crates/tiramemsu/tests/native_runner.rs`; the differential corpus and the property test cover answers.
- Benchmarks: `crates/tm-bench/examples/probe.rs` keeps loaded files between runs for tuning.

## Results

The full backend benchmark (`bench/backends/README.md`, every answer compared across engines before timing), redb against the 0.8.0 run:

| Query | 10⁵ before → after | 10⁶ before → after | SQLite at 10⁶ |
|---|---|---|---|
| F1 | 13.1 → 6.7 ms | 83.1 → 20.2 ms | 23.2 ms |
| C1 | 5.08 → 2.04 ms | 34.7 → 20.7 ms | 13.4 ms |
| S3 | 690 → 372 µs | 6.00 → 3.84 ms | 2.23 ms |
| C2 | 13.0 → 7.3 ms | 166 → 96 ms | 87 ms |
| Cypher 2-hop | 5.33 → 0.32 ms | 47.0 → 0.38 ms | 14.4 ms |
| skewed triangle | 9.7 → 4.7 ms | 44.9 → 20.8 ms | 4.87 s |
| skewed 4-cycle | 3.91 → 1.41 s | timeout → 37.6 s | timeout |

Load, size and writes are unchanged; variable-length Cypher paths remain about 2.5× slower than SQLite and are left for a change to the path engine.
