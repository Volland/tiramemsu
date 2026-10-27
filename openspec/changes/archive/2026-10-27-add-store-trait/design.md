## Context

`tm-core` reaches its file through `Executor`, a SQL-text interface ([[architecture#Executor]]). An audit of every statement `tm-core` issues (2026-10-27) shows the transaction engine and the read paths need only a handful of access shapes:

- point lookups: a statement by eid, a term by id or by key, a transaction instant, a meta counter, a volatile value, an embedding;
- prefix scans over live statements with any subset of `s`, `p`, `o` bound, optionally in eid order (idempotency, cascade children, cardinality, unique, schema checks, graph membership, conflicts);
- the same scans over all statements, live or retracted, filtered by a view (`View::triples`, bundles, recall);
- the log: statements added or retracted after a transaction (`events_since`), instants to transactions (`asOf` by instant);
- appends: a term, a transaction, a statement, a single retraction, a counter update, a `pred_multi` row, text postings, an embedding; and one non-append, the volatile upsert/delete.

That is a small, typed contract. The Lean model in `paper/` already splits it into `ReadStore` and `WriteStore`.

## Goals / Non-Goals

**Goals:** one transaction engine with the exact semantics of today's engine; a storage contract that a key-value store can implement with prefix range scans; an executable reference implementation; a differential oracle against the SQLite engine.

**Non-goals:** query execution (IR) over the contract, redb, any change to the SQLite engine.

## Decisions

### D-1. Two traits named after the mechanised model

```rust
pub trait ReadStore {
    fn meta(&self) -> Result<Meta>;                              // counters and format facts
    fn term(&self, id: u64) -> Result<Option<TermRow>>;
    fn term_id(&self, key: &TermKey) -> Result<Option<u64>>;
    fn tx_instant(&self, t: u64) -> Result<Option<i64>>;
    fn tx_at_or_before(&self, instant: i64) -> Result<u64>;     // 0 when none
    fn statement(&self, eid: u64) -> Result<Option<StmtRow>>;
    fn scan(&self, pat: Pattern, scope: Scope) -> Result<RowIter<'_>>;   // see D-3
    fn count_hint(&self, pat: Pattern, scope: Scope, cap: u64) -> Result<u64>;
    fn added_after(&self, t: u64) -> Result<RowIter<'_>>;       // t_add > t, by (t_add, eid)
    fn retracted_after(&self, t: u64) -> Result<RowIter<'_>>;   // t_ret > t, by (t_ret, eid)
    fn pred_multi(&self, p: u64) -> Result<bool>;
    fn volatile(&self, s: u64, key: u64) -> Result<Option<VolatileRow>>;
    fn text_postings(&self, token: &str) -> Result<PostingIter<'_>>; // add-redb-backend fills it in
    fn embedding(&self, model: u64, eid: u64) -> Result<Option<EmbeddingRow>>;
    // ...listing helpers for embeddings and models
}

pub trait WriteStore: ReadStore {
    fn put_meta(&mut self, m: &Meta) -> Result<()>;
    fn insert_term(&mut self, id: u64, row: &TermRow) -> Result<()>;            // fails if id exists
    fn insert_tx(&mut self, t: u64, instant: i64) -> Result<()>;                // fails if t exists
    fn insert_statement(&mut self, row: &StmtRow) -> Result<()>;                // fails if eid exists
    fn retract(&mut self, eid: u64, t: u64, kind: RetKind) -> Result<bool>;     // live -> retracted, once
    fn add_pred_multi(&mut self, p: u64) -> Result<bool>;
    fn put_volatile(&mut self, s: u64, key: u64, row: &VolatileRow) -> Result<()>;
    fn delete_volatile(&mut self, s: u64, key: u64) -> Result<bool>;
    fn index_text(&mut self, value: u64, text: &str, lang: Option<&str>) -> Result<()>;
    fn insert_embedding(&mut self, row: &EmbeddingRow) -> Result<()>;
}
```

Rejected: one trait with a SQL-like query method (it would recreate `Executor`); a trait per table (the engine's operations span tables, and the backend needs to see a whole statement write to update all its permutations at once).

- motivated_by:: Lean `ReadStore`/`WriteStore` (`paper/statement-as-one-record/`) and the access-shape audit above.

### D-2. Rows are plain records of ObjectId raws

`StmtRow { eid, s, p, o, t_add, t_ret: Option<u64>, v_from: Option<i64>, v_to: Option<i64>, ret_kind: Option<RetKind> }`, `TermRow { tag, lex, dt, lang, num }`. The engine converts to `ObjectId`/`Eid` at the edge. Values are the same integers the SQLite file holds, so a `MemStore` or redb file and a SQLite file of the same history are equal row for row, which is what makes `tm convert` (add-redb-backend) lossless and the oracle test (D-6) exact.

### D-3. One scan with a pattern and a scope

`Pattern { s, p, o: Option<u64> }`, `Scope::{Live, All}`, and an ordering request `Order::{Any, Eid}`. A backend picks the permutation whose prefix covers the bound positions; `Order::Eid` asks for ascending eid within the result, which the engine needs where the SQLite engine wrote `ORDER BY eid` (deterministic idempotency winner, cascade order, cardinality replacement order). Backends that keep the eid as the last key component return eid order for free when all three positions are bound; otherwise the engine sorts the (small) result. Valid-time and view filtering stay in the engine, as pure functions over rows, so their semantics live in one place.

`count_hint` returns the number of matches up to `cap` (exact below it) for planning; `MemStore` counts, redb uses its statistics and bounded probes.

### D-4. The engine is a port of `tm-core`'s engine, not a redesign

`tm_store::Tx` keeps every check, error and ordering of `tm_core::Tx`: the pre-insert pipeline (positions, reserved namespace, interval, value and subject type, flag validation, idempotency, unique, cardinality-one, self-reference, live endpoints), the BFS cascade in eid order per parent, supersede's dropped-membership propagation and σ allocation order, the transaction-date guards, burned ids. Each SQL statement in `tm-core/src/engine/` maps to a trait call plus, at most, an in-memory filter. The port is reviewed file by file against the original, and the oracle test (D-6) checks it.

- motivated_by:: [[overview#Decision Record]] D5, D8, D9, D17 (the semantics must not change with the backend).

### D-5. Speculation without nested savepoints

`speculate` and `dry_run` run on a write transaction that is then aborted; the burned counters are then written in a second, small write transaction. This is the same sequence the SQLite engine runs (its savepoint is taken on a clean transaction and rolled back whole), and it needs nothing beyond begin/commit/abort, which every KV store has (redb's ephemeral savepoints are refused once a transaction is dirty).

- supersedes:: the SQLite engine's `SAVEPOINT spec` (only for backends on the trait).

### D-6. The SQLite engine is the oracle

A proptest generator produces sequences of operations (assert, create, confirm, retract, retract-matching, supersede with patches, memberships, schema flags, volatile, dry runs, speculations, failing bodies). Each sequence runs on `tm_core::Store` over `tm-rusqlite` and on `tm_store::Engine` over `MemStore`; after every transaction the test compares the `TxReport`s (or the error kinds), and at the end the full statement table, terms, transactions, counters, `pred_multi` and `events_since(0)`.

### D-7. Never forget by construction

`WriteStore` has no delete or rewrite for statements, terms or transactions. `insert_*` fail on an existing key (`Error::Corrupt` naming the key), and `retract` changes only a live statement. Volatile state keeps its delete: it is outside the graph by design ([[storage#Volatile Table]]).

- motivated_by:: [[overview#Decision Record]] D17.

## Constraints

- **Same-file semantics:** a backend on the trait stores the same integers as format 4 of the SQLite file; anything that differs breaks conversion and the oracle. Constrains [[openspec:storage-trait#Engine equivalence with SQLite]] {strength: must}.
- **No unsafe:** the workspace forbids `unsafe_code`; `tm-store` and its backends inherit it.

## Risks / Trade-offs

- **Porting drift:** two engines exist until `move-sqlite-onto-store`. Mitigated by the oracle test, which runs in CI on every change to either engine.
- **Scan cost on the in-memory store:** irrelevant for production, since `MemStore` is a reference; its maps are keyed like redb's permutations, so it exercises the same prefix logic.
- **Sorting for `Order::Eid`:** on large unbound scans this is O(n log n). The engine only asks for eid order on scans bounded by at least one position, as the SQLite engine does.

## Open Questions

None. Naming follows the interview; exact field names are fixed in the code.

## Resolution

Implemented 2026-10-27.

- **Final names.** `ReadStore::{meta, term, term_id, tx_instant, tx_at_or_before, statement, scan, scan_eid, count, added_since, retracted_since, pred_multi, pred_multi_all, volatile, embedding, embeddings, text_doc, text_postings, text_tokens_with_prefix, text_stats}`; `WriteStore::{set_meta, insert_term, insert_tx, insert_statement, retract, add_pred_multi, put_volatile, delete_volatile, insert_embedding, text_insert, text_clear}`; `WriteTxn::{commit, abort}`; `Backend::{name, read, write}`. Eid order is a method (`scan_eid`) rather than an `Order` argument; `count` (exact up to a cap) replaces `count_hint`.
- **Logical format.** A trait store's `meta.format_version` is the SQLite logical format (5), so a store says which rows it holds; a backend keeps its physical layout version under its own key.
- **Shared algorithms instead of copies** where the data access is small: bundle export runs on both paths through `tm_core::BundleSource`. Conflict inspection and the evidence reader are ported (`tm_store::conflict`, `tm_store::evidence`) and checked by the oracle.
- **Text index.** `tm_store::text` tokenises like FTS5 `unicode61 remove_diacritics 2` (diacritics removed from Latin letters only) and indexes inside the writing transaction; search arrives with `add-redb-backend`.
- **Errors.** Backend failures are the new `Error::Storage { message }` (bridge code `Storage`).
- **Evidence.** The oracle (`crates/tm-store/tests/oracle.rs`) passed 3 000 random histories; a planted cascade-order bug fails it within a few cases. The contract checks (`tm_store::contract`) are reusable by every backend.
