## 1. Engine support

- [x] 1.1 `tm-datalog`: inline non-recursive derived relations into constraint bodies; classify `error` constraints as enforceable or not (`ConstraintNotEnforceable`).
- [x] 1.2 Delta-seeded variants per positive atom (added rows) and per negated atom (retracted rows), with seed time spans widened by metric windows.
- [x] 1.3 Pre-state evaluation of found violations (post-state minus added plus retracted) to drop pre-existing ones.
- [x] 1.4 Seeded-check entry point taking the delta rows, the store and a budget, returning violations and seed-row counts.

## 2. Facade

- [x] 2.1 `TxDatalog::enforce(program, on)` with the full check on enable and on supersede of an enforced program's text.
- [x] 2.2 Enforced programs are read from the transaction's own state at each commit (the `dl:enforced` marks and the program text), so no cache can go stale; a hook with no marks costs one dictionary lookup (design DL16 refined: cache not needed).
- [x] 2.3 Pre-commit hook in `Db::transact_leased` (`src/datalog_enforce.rs`): delta scan, predicate filter, seeded checks, `ConstraintViolated` rollback, `warn` violations in the report.
- [x] 2.4 `OpenOptions::constraint_budget`, `TxOptions::constraint_budget`, `ConstraintBudgetExceeded`; `TxReport::constraint_checks`, `seed_rows`, `constraint_violations`.
- [x] 2.5 Dry-run collection of violations; `with` fails as a commit.
- [x] 2.6 Annotate implementations with `@tg: implements:: [[openspec:datalog-constraints#<Requirement>]]`.

## 3. Surfaces

- [x] 3.1 Bridge tx op `enforce`, report fields, error payload with violations.
- [x] 3.2 Node, Python, wasm wrappers.
- [x] 3.3 MCP write tool `enforce_program` (hidden in read-only mode).
- [x] 3.4 CLI `tm program enforce|unenforce`; `tm program show` reports enforcement.

## 4. Tests

- [x] 4.1 Scenario tests for every scenario of the delta spec on SQLite and redb, annotated `@tg: verifies:: [[openspec:datalog-constraints#<Requirement>#<Scenario>]]`.
- [x] 4.2 Property: on random histories with random constraints, a transaction is refused by the seeded check exactly when full `View::check` after it reports a violation that full `check` before it did not.
- [x] 4.3 Property: SQLite and redb refuse the same transactions with the same violations.
- [x] 4.4 Benchmark: commit overhead of enforced constraints against transaction size and store size (gate ≤ 20 % on a 100-statement transaction at 10⁶).

## 5. Documentation

- [x] 5.1 `lat.md/datalog.md` DL15 and DL16 implemented with code links; `- constrains:: [[datalog#Warn And Error Constraints]] {strength: must}` from [[constraints#Single Writer]]; add `datalog-constraints` enforcement to the file's `openspec:` frontmatter if missing.
- [x] 5.2 `lat.md/architecture.md#Connections and Concurrency`, `lat.md/data-model.md#Predicate Schema`, `lat.md/bindings.md`, `lat.md/api.md`; CHANGELOG (the site page section is not written).
- [x] 5.3 `tg check`, `tg ontology check` and `tg trace datalog-constraints --gaps` show no new gaps; `openspec validate add-datalog-constraints` passes.
