## 1. Shared core

- [ ] 1.1 Extract `crates/tiramemsu/src/materialize.rs` from `n3_materialize.rs`: conclusion list in proof order, stale-support closure, existing and widening detection, `derive`, skolems, layers, labels.
- [ ] 1.2 Reduce `n3_materialize.rs` to the N3 adapter; all N3 materialization tests pass unchanged.

## 2. Datalog materialization

- [ ] 2.1 `dl:derivedBy`, `dl:rule`, `dl:skolem`, `dl:materialized`, `dl:reconciled`, `dl:reconciledCount` in `crates/tm-core/src/vocab.rs`.
- [ ] 2.2 `Db::materialize_program` with `MaterializeProgramOptions` and `MaterializeProgramReport`; snapshot evaluation excluding own conclusions; fragments with one derivation each; unstorable classification.
- [ ] 2.3 Reconciliation of non-monotonic strata with transaction labels and `needs_rerun_after_writes`.
- [ ] 2.4 Invented nodes through `upsert_derived(dl:skolem, key, support)`.
- [ ] 2.5 Incremental runs: base transaction, base relations, added-rows delta in `tm-datalog`, rederivation by demand goals with the 10 % fallback; `full`.
- [ ] 2.6 Annotate implementations with `@tg: implements:: [[openspec:datalog-materialization#<Requirement>]]`.

## 3. Surfaces

- [ ] 3.1 Bridge database op `materializeProgram`.
- [ ] 3.2 Node, Python, wasm wrappers.
- [ ] 3.3 MCP write tool `materialize_program` (hidden in read-only mode).
- [ ] 3.4 CLI `tm materialize-program`.

## 4. Tests

- [ ] 4.1 Scenario tests for every scenario of the delta spec on SQLite and redb, annotated `@tg: verifies:: [[openspec:datalog-materialization#<Requirement>#<Scenario>]]`.
- [ ] 4.2 Property: grounding — after materialization and random retract and supersede sequences, every live conclusion's support is live.
- [ ] 4.3 Property: incremental equals full on random interleavings of writes and runs.
- [ ] 4.4 Property: after reconciliation, the program's live conclusions equal `datalog_reason` on the commit's snapshot (content and intervals).
- [ ] 4.5 Benchmark: incremental run cost against the size of the change at 10⁶ statements.

## 5. Documentation

- [ ] 5.1 `lat.md/datalog.md`: DL13, DL14, DL21, DL22 implemented with code links; the DL22 refinement (transaction label) recorded with a `supersedes::` edge; `datalog-materialization` in the `openspec:` frontmatter.
- [ ] 5.2 `lat.md/reasoning.md#Monotonic Materialization` (reconciliation exists for Datalog), `lat.md/data-model.md#Derived Beliefs`, `lat.md/bindings.md`, `lat.md/api.md`; CHANGELOG; a recipe with its test.
- [ ] 5.3 `tg check`, `tg ontology check` and `tg trace datalog-materialization --gaps` show no new gaps; `openspec validate add-datalog-materialization` passes.
