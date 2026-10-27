# Native query runner

Target release: **0.8 — Two engines**. Third of six changes (D42–D43 in `lat.md/overview.md#Decision Record`). Depends on `add-store-trait` and `add-redb-backend`.

## Why

The IR ([[query#Logical IR]]) is the boundary both front ends compile to, and today its only executor turns it into SQL. A redb database has no SQL, so SPARQL and Cypher need an executor that reads the storage traits directly. That executor is also where the performance case for redb is made or lost: sorted permutations in every order make worst-case-optimal joins cheap, exact statistics make join ordering reliable, and snapshot readers make concurrent queries free of a pool.

## What Changes

- **New crate `tm-native`**: an executor for the whole IR over `ReadStore`, producing the same `QueryResult` as `tm-exec`.
- **Planner:** cost-based join ordering by dynamic programming over connected subsets (greedy above 12 patterns), with cardinalities from exact predicate statistics and bounded range probes; per-pattern time scopes and graph selections become access-path filters.
- **Hybrid join engine** (design interview Q4, after Kùzu and Umbra):
  - acyclic parts: pipelined index nested-loop joins on permutation prefixes when the probe side is bound, otherwise hash joins with semijoin (Bloom) filters passed sideways;
  - cyclic parts: a worst-case-optimal multiway intersection (generic join) over sorted prefix ranges, seeking with redb cursors and switching to an in-memory sorted trie when a range is reused;
  - batched execution over vectors of ObjectIds; terms are decoded only for projected and filtered columns, with a per-query decode cache.
- **Every IR operator:** triple and path patterns, text, vector and hybrid recall patterns, `Values`, `Unnest`, `Join`, `LeftJoin`, `Filter`, `Union`, `Extend`, `Aggregate`, `Project`, `OrderLimit`, `RowNumber`, virtual predicates, provenance, SPARQL and Cypher semantics flags (set vs bag, isomorphism).
- **Paths:** the existing path engine (automaton, trail, time-respecting search, completeness) runs over a store-backed neighbour fetch instead of SQL.
- **Parallelism:** scans and hash builds above an estimated 50 000 rows split into morsels on a thread pool; small queries stay on the calling thread.
- **Budgets and explain:** query budgets (deadline, cancel, rows, bytes) are polled per batch; `explain` describes the native plan (join order, join kind per step, estimates).
- **Writes from query languages:** SPARQL Update and Cypher writes run on redb through the shared engine.
- **The facade routes** every query on a redb database to `tm-native`. On SQLite, `tm-exec` stays the default; `OpenOptions::runner = Runner::Native` runs SQLite queries on `tm-native` over the SQLite store once `move-sqlite-onto-store` lands.

## Capabilities

### New Capabilities

- `native-query-execution`: the native executor, its planner, join strategies, parallelism, budgets and explain.

### Modified Capabilities

- `dialect-differential-testing`: the differential corpus also runs on the native runner and requires identical results (ADDED requirement).

## Impact

- **New crate:** `crates/tm-native` (`plan/`, `join/`, `expr.rs`, `path.rs`, `recall.rs`, `exec.rs`, `explain.rs`).
- **Shared code moved out of `tm-exec`:** the Rust implementations of SPARQL/Cypher functions (`udf_fn.rs`) and result decoding (`decode.rs`, `result.rs`) become usable without SQLite (a `pure` module or a small `tm-func` crate); the path automaton and search take a neighbour-fetch trait.
- **Facade:** `view.rs`, `sparql.rs`, `cypher.rs` route by backend; `OpenOptions::runner`.
- **Tests:** the SPARQL and Cypher differential suites, W3C SPARQL tests, recipes and path suites run on redb.
- **lat.md:** `query.md` new section Native Runner; `architecture.md`; `tests.md`.
- **Requirements added:** [[openspec:native-query-execution#Same answers as the SQL runner]], [[openspec:native-query-execution#Cost-based join order]], [[openspec:native-query-execution#Worst-case-optimal cyclic joins]], [[openspec:native-query-execution#Late decoding]], [[openspec:native-query-execution#Parallel scans]], [[openspec:native-query-execution#Budgets on the native runner]], [[openspec:native-query-execution#Native explain]].

## Non-goals

- No change to SQL routing on SQLite files.
- No new query syntax.
- No Free Join; the plan form keeps that option open.
