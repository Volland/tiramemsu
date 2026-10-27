# Storage trait and one transaction engine over it

Target release: **0.8 — Two engines**. First of six changes from the storage-backend design interview of 2026-10-27 (decisions D37–D44 in `lat.md/overview.md#Decision Record`).

## Why

Every layer below the IR speaks SQL: the `Executor` trait in `tm-core` takes SQL text, the transaction engine is about 80 statements, and `tm-exec` generates SQL for every plan. A second storage engine (redb, `add-redb-backend`) therefore cannot be another `Executor` host. It needs a typed storage interface whose operations are what the engine means (find the live statement with this `(s, p, o)`, list the dependents of an eid, append to the log), not SQL strings.

The semantics that carry the paper's guarantees (idempotent assert, the grounded cascade, supersede replay, live endpoints, burned ids) must exist once. If each backend kept its own engine, every later fix to cascade or supersede would have to be made twice and proven twice.

## What Changes

- **New crate `tm-store`** with the typed storage traits `ReadStore` (one snapshot) and `WriteStore` (one write transaction), named after the Lean classes of the mechanised model (`paper/`). Keys and values are `u64`/`i64` ObjectIds and plain records; there is no SQL anywhere in the crate.
- **One transaction engine over the traits** (`tm_store::Engine` and its write handle `tm_store::Tx`), with the same operations, errors and report as `tm_core::Tx`: `assert`, `create`, `confirm`, `retract`, retract by pattern, `supersede`, graph membership, typed layers and the predicate schema, cardinality and uniqueness, volatile state, fact bundles, embeddings, text-index upkeep, speculation (`with`), dry runs and burned ids.
- **A term dictionary over the traits** with the same encoding (`term` ids, inline values, `SHORT_STR`) and a committed-term LRU cache.
- **Read operations over the traits**: views (`now`, `asOf`, `history`, valid time), `View::triples`, graph listing, volatile values, `events_since`, `dependents`, conflict inspection.
- **A reference in-memory store** (`tm_store::mem::MemStore`, ordered maps behind one lock) that implements both traits. It is the executable specification of the trait contract and the fast backend for property tests.
- **The SQLite engine becomes the oracle**: a differential property test runs random operation sequences on the existing SQLite engine and on the new engine over `MemStore`, and requires identical eids, transaction numbers, reports, histories, event logs and errors.
- **Never-forget by construction**: `WriteStore` has no operation that deletes or rewrites a statement, term or transaction; the only mutation of a statement is setting its retraction once. The SQLite triggers stay for the SQLite file.

## Capabilities

### New Capabilities

- `storage-trait`: the typed storage contract, the in-memory reference store, the shared transaction engine and its equivalence with the SQLite engine.

### Modified Capabilities

None. The SQLite engine and every public signature of `tm-core` and `tiramemsu` are unchanged; `move-sqlite-onto-store` replaces the SQLite engine later.

## Impact

- **New crate:** `crates/tm-store` (`traits.rs`, `keys.rs`, `mem.rs`, `dict.rs`, `engine/` mirroring `tm-core/src/engine/`, `read.rs`, `text.rs`, `vector.rs`).
- **`tm-core`:** pure modules (`id`, `codec`, `value`, `vocab`, `report`, `error`, `clock`, `budget`, `rdf`, `mapping`, `bundle` data types) are reused as they are; a few private helpers become `pub` (`Counters` fields, `text::tokenize`, schema flag parsing). No behaviour change.
- **Tests:** `crates/tm-store/tests/oracle.rs` (differential property test against `tm-core` on `tm-rusqlite`), plus the scenario tests below.
- **lat.md:** `architecture.md` (new section Storage Backends, Crates), `overview.md` (Decision Record rows D37–D44), `tests.md` (new Storage Trait section).
- **Decisions that bind the area:** [[overview#Decision Record]] D5 (idempotent assert), D8 (cascade), D9 (one row per occurrence, ids never reused), D17 (never forget), D18 (speculation), D35 (origin bits).
- **Requirements added:** [[openspec:storage-trait#Typed storage operations]], [[openspec:storage-trait#One engine for every backend]], [[openspec:storage-trait#Engine equivalence with SQLite]], [[openspec:storage-trait#Never forget by construction]], [[openspec:storage-trait#Speculation without nested savepoints]], [[openspec:storage-trait#Snapshot reads]].

## Non-goals

- No redb code (that is `add-redb-backend`) and no query execution over the traits (`add-native-runner`).
- No change to the SQLite engine, its file format or its performance.
- No user-visible API change.
