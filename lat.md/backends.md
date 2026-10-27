---
type: Decision
---
# Storage Backends

Decisions D37–D44 of the storage-backend design interview of 2026-10-27: a second storage engine on redb with its own native query runner, beside SQLite.

The work is six OpenSpec changes, built and archived in order: `add-store-trait`, `add-redb-backend`, `add-native-runner`, `add-tm-cli`, `add-backend-benchmarks` and `move-sqlite-onto-store`. Option analysis that came before is in `docs/storage-engine-research.md`.

## Typed Storage Trait

D37: a typed storage contract (`ReadStore`, `WriteStore`) in a new crate `tm-store`, and one transaction engine over it, instead of making redb another SQL `Executor` host.

The SQLite path keeps its SQL code generation; redb gets a native executor below the IR. The trait names and the row conventions follow the Lean model of [[architecture#Store Abstraction]].

- motivated_by:: [[constraints#Same Logical Database]] {weight: primary, since: "2026-10-27"}

## Same Logical Database

D38: a redb file holds the same logical rows as a SQLite file with the same history: ObjectIds, eids, both clocks, retraction kinds, terms, counters and burned ids, the event log, text and vector indexes.

Conversion between backends is lossless, the differential tests compare exact eids, and benchmarks compare like with like. Text recall on redb uses its own inverted index that reproduces FTS5's tokeniser and BM25 constants.

- motivated_by:: [[constraints#Same Logical Database]] {weight: primary, since: "2026-10-27"}
- motivated_by:: [[backends#Typed Storage Trait]] {weight: secondary}

## Six Live Permutations

D39: redb keeps live statements in all six `(s, p, o)` orders and all statements in three history orders, plus exact per-predicate statistics updated in each write transaction.

Every variable order a multiway join can choose then has a matching key prefix, and the planner knows cardinalities without `ANALYZE`. The price is about eleven B-tree writes per assert, which the benchmarks measure.

- motivated_by:: [[backends#Hybrid Join Engine]] {weight: primary, since: "2026-10-27"}

## Durability Modes

D40: one `durability` option for both backends: `Normal` (default; redb group commit, SQLite `synchronous = NORMAL`), `Immediate` (every commit fsynced) and `None` (durable only on sync and close).

A crash never corrupts either file in any mode; the modes differ only in how many of the newest commits may be lost.

- motivated_by:: [[constraints#Crash Consistency]] {weight: primary, since: "2026-10-27"}

## One Engine In Two Phases

D41: phase 1 runs the new trait engine on redb with the untouched SQLite engine as its oracle; phase 2 (`move-sqlite-onto-store`) moves SQLite onto the trait, gated on no more than 5 % slowdown.

- motivated_by:: [[backends#Typed Storage Trait]] {weight: primary, since: "2026-10-27"}

Phase 1 is done. Phase 2 was measured on 2026-10-27: the trait engine on SQLite (`SqlBackend`) writes the same rows as the SQL engine, but cardinality replacement is 70 % slower and an annotated retraction 91 % slower, while supersede is 6× faster. The gate is not met, so SQLite still writes through the SQL engine; the change was closed without landing; the two engines stay equal through the oracle, which also runs the trait engine on SQLite.

## Hybrid Join Engine

D42: the native runner joins acyclic parts with index nested loops and hash joins, and cyclic cores with a worst-case-optimal multiway intersection, in an order chosen greedily from exact predicate statistics.

This is the Kùzu and Umbra design; [[query#Native Runner]] describes the operators. Two parts of the plan were deferred after measurement: dynamic-programming join order (greedy with exact statistics matched it on the corpus) and sideways semijoin filters. MillenniumDB's LFTJ-everywhere is simpler but slower on the chains and stars that dominate agent memory; Free Join is newer research with no production precedent. See [[query#Physical Planning#LFTJ]] for the measured cyclic-join gains on SQLite.

- motivated_by:: [[constraints#Read Performance First]] {weight: primary, since: "2026-10-27"}

## Snapshot Readers

D43: on redb every view is one MVCC read transaction, with no reader pool and no limit on concurrent views; join steps over more input rows than `parallel_threshold` run as parallel morsels.

Scans and hash builds stay sequential: a redb range cursor is sequential, and splitting key ranges is left for when a benchmark shows it matters.

- motivated_by:: [[constraints#Read Performance First]] {weight: primary, since: "2026-10-27"}

## Backend Detection and CLI

D44: `Db::open` detects the backend from the file header, `Backend::{Auto, Sqlite, Redb}` chooses it for new files, and SQLite stays the default.

An explicit choice that disagrees with an existing file fails with `BackendMismatch`. The `tm` CLI creates, inspects, queries, imports, converts, compacts and benchmarks files.

- motivated_by:: [[backends#Same Logical Database]] {weight: secondary, since: "2026-10-27"}
