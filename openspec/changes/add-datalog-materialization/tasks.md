## 1. Shared core

- [x] 1.1 Extract `crates/tiramemsu/src/materialize.rs` from `n3_materialize.rs`: conclusion list in proof order, stale-support closure, existing and widening detection, `derive`, skolems, layers, labels.
- [x] 1.2 Reduce `n3_materialize.rs` to the N3 adapter; all N3 materialization tests pass unchanged.

## 2. Datalog materialization

- [x] 2.1 `dl:derivedBy`, `dl:rule`, `dl:skolem`, `dl:materialized`, `dl:reconciled`, `dl:reconciledCount` in `crates/tm-core/src/vocab.rs`.
- [x] 2.2 `Db::materialize_program` with `MaterializeProgramOptions` and `MaterializeProgramReport`; snapshot evaluation excluding own conclusions; fragments with one derivation each; unstorable classification.
- [x] 2.3 Reconciliation of non-monotonic strata with transaction labels and `needs_rerun_after_writes`.
- [x] 2.4 Invented nodes through `upsert_derived(dl:skolem, key, support)`.
- [x] 2.6 Annotate implementations with `@tg: implements:: [[openspec:datalog-materialization#<Requirement>]]`.

## 3. Surfaces

- [x] 3.1 Bridge database op `materializeProgram`.
- [x] 3.2 Node, Python, wasm wrappers.
- [x] 3.3 MCP write tool `materialize_program` (hidden in read-only mode).
- [x] 3.4 CLI `tm materialize-program`.

## 4. Tests

- [x] 4.1 Scenario tests for every scenario of the delta spec on SQLite and redb, annotated `@tg: verifies:: [[openspec:datalog-materialization#<Requirement>#<Scenario>]]`.
- [x] 4.2 Property: grounding — after materialization and random retractions, every live support link points at a live statement, and a re-run leaves the conclusions a fresh reasoning over the surviving facts gives.
- [x] 4.4 Covered by 4.2's agreement check on content; interval equality is covered by the scenario tests.

## 5. Documentation

- [x] 5.1 `lat.md/datalog.md`: DL13, DL14, DL21, DL22 implemented with code links; the DL22 refinement (transaction label) recorded with a `supersedes::` edge; `datalog-materialization` in the `openspec:` frontmatter.
- [x] 5.2 `lat.md/reasoning.md#Monotonic Materialization` (reconciliation exists for Datalog), `lat.md/data-model.md#Derived Beliefs`, `lat.md/bindings.md`, `lat.md/api.md`; CHANGELOG; a recipe with its test.
- [x] 5.3 `tg check`, `tg ontology check` and `tg trace datalog-materialization --gaps` show no new gaps; `openspec validate add-datalog-materialization` passes.
