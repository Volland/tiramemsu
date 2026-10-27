## Status

Implemented 2026-10-27. Depends on `add-store-trait` and `add-redb-backend`. See the Resolution in `design.md` for where the implementation departs from the design.

## 1. Shared pieces

- [x] 1.1 The function library and result decoding are reached without SQLite: `tm-native` calls the `tm-exec` UDF registries directly, and the `Decoder` loads terms through `TermLoad` (design D-4).
- [x] 1.2 `NeighbourFetch` for the path search and `PathSource` for the path engine; the SQL implementations are unchanged (design D-5).

## 2. Planner

- [x] 2.1 Cardinality estimation from exact predicate statistics; greedy join order; cyclic-core detection — `@tg: implements:: [[openspec:native-query-execution#Cost-based join order]]`.
- [x] 2.2 Access-path selection per pattern: the permutation is chosen by the store from the bound positions and named in explain (`ReadStore::access_path`).

## 3. Operators

- [x] 3.1 Scans, index nested loop and hash join on ObjectIds — `@tg: implements:: [[openspec:native-query-execution#Late decoding]]`. Bloom filters deferred (Resolution).
- [x] 3.2 Generic join over hash tries — `@tg: implements:: [[openspec:native-query-execution#Worst-case-optimal cyclic joins]]`.
- [x] 3.3 `Values`, `Unnest`, `LeftJoin`, `Filter`, `Union`, `Extend`, `Aggregate`, `Project`, `OrderLimit`, `RowNumber`, virtual predicates, provenance, semantics flags — `@tg: implements:: [[openspec:native-query-execution#Same answers as the SQL runner]]`.
- [x] 3.4 Paths, text, vector and hybrid recall patterns.
- [x] 3.5 Morsel parallelism for join steps — `@tg: implements:: [[openspec:native-query-execution#Parallel scans]]` (partial: scans and hash builds stay sequential).
- [x] 3.6 Budgets and explain — `@tg: implements:: [[openspec:native-query-execution#Budgets on the native runner]]`, `[[openspec:native-query-execution#Native explain]]`.

## 4. Facade

- [x] 4.1 SPARQL, Cypher, `execute_ir`, `explain_*`, paths and writes from query languages on redb run on `tm-native`; `OpenOptions::parallel_threshold`. No `OpenOptions::runner`: the backend decides the runner (Resolution).

## 5. Tests

- [x] 5.1 The dialect corpus and an extra SPARQL corpus on both backends (`tests/native_runner.rs`), and the facade suites on redb (`TIRAMEMSU_DEFAULT_BACKEND=redb`).
- [x] 5.2 Random-query property test; triangle AGM-bound test; count-without-decode test; cancellation; explain; parallel equality; rare predicate first; acyclic stays binary.

## 6. Documentation and checks

- [x] 6.1 lat.md: `query.md` section Native Runner with the join strategy research summary; decisions D42–D43 corrected to what was built.
- [x] 6.2 `cargo test --workspace --features tiramemsu/redb`, clippy, `tg check`, `tg ontology check`, `tg trace native-query-execution --gaps`, `openspec validate add-native-runner`.
