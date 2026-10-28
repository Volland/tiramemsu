# Materialize Datalog programs as grounded, reconciled beliefs

Source: decisions DL13, DL14, DL21 and DL22 of `lat.md/datalog.md` (interview of 2026-10-28). Change 3 of four; depends on `add-datalog-engine` and on the derived beliefs of `add-derived-beliefs` (archived).

## Why

A goal query answers once; memory should keep what follows, and drop it when its grounds go. N3 already materializes conclusions as derived beliefs ([[reasoning#Rules Are Support]]), but only for monotonic rule sets ([[reasoning#Monotonic Materialization]]) and by recomputing everything on each run. Datalog programs use negation, aggregates and temporal operators that N3 refuses to materialize, and agent memories change a little at a time, so a run should write only what changed.

## What Changes

- **`Db::materialize_program(program, &MaterializeProgramOptions)`:** evaluates the live program on a snapshot and writes the conclusions whose predicate is a stored-predicate IRI through `derive`, in proof order, with support = premise eids + eids written for derived premises + the program statement, and layers `(D dl:derivedBy program)` and `(D dl:rule i)`. Each maximal interval fragment with one support becomes one statement; existing conclusions with the same content and support are kept or widened by supersede; invented nodes become skolem nodes through `upsert_derived(dl:skolem, key, support)`. Conclusions that a statement cannot hold (a literal subject, a plain-named relation) are reported as `unstorable`.
- **Reconciliation:** strata that use negation or aggregates are materialized too. A run retracts the program's live conclusions of those strata that the run no longer derives, through ordinary retraction, and labels the transaction `(tx dl:reconciled program)` with a count. The report marks such strata `needs_rerun_after_writes`, since a later write can invalidate them before the next run.
- **Shared with N3:** the snapshot-then-commit, proof-order derive, stale-support skipping, widening and skolem logic move from `n3_materialize.rs` into a module both languages use, with N3 behavior unchanged.
- **Not in this change:** incremental runs (DL22) need the engine to evaluate from a delta of added statements over the program's existing conclusions; they are recorded as open in `lat.md/datalog.md` and every run here is a full run that writes only the difference.
- **Surfaces:** bridge database op `materializeProgram`; Node, Python, wasm; MCP write tool `materialize_program` (hidden in read-only mode); CLI `tm materialize-program <file> <program> [--dry-run] [--require-complete] [--window from..to] [--graph iri]…`.

## Capabilities

### New Capabilities

- `datalog-materialization`: materializing programs through `derive`, support with the program statement, valid-time fragments, widening, invented nodes, reconciliation , snapshot then commit, and the surfaces.

### Modified Capabilities

None. `n3-materialization` behavior is unchanged by the shared module; its scenario tests guard it.

## Impact

- **`crates/tiramemsu`:** new `src/materialize.rs` (shared core), `src/n3_materialize.rs` reduced to the N3 adapter, `src/datalog_materialize.rs`; `MaterializeProgramOptions`, `MaterializeProgramReport`.
- **`crates/tm-datalog`:** evaluation seeded from a base relation and an added-rows delta; rederivation checks by goal; stratum monotonicity in reports.
- **`crates/tm-core`:** `dl:derivedBy`, `dl:rule`, `dl:skolem`, `dl:materialized`, `dl:reconciled` vocabulary.
- **Bindings, MCP, CLI** as listed.
- **lat.md:** `lat.md/datalog.md` (DL13, DL14, DL21, DL22 implemented; DL22's record moves to a transaction label), `lat.md/reasoning.md#Monotonic Materialization` (reconciliation exists for Datalog), `lat.md/data-model.md#Derived Beliefs`, `lat.md/bindings.md`, `lat.md/api.md`.
- **Binding decisions and constraints:** [[constraints#Grounded Memory]], [[constraints#Never Forget]], [[constraints#Single Writer]], [[reasoning#Snapshot Then Commit]], [[reasoning#Merge By Support]], [[reasoning#Widening]], [[reasoning#Skolem Nodes]], [[derived-beliefs#Idempotent Per Support Set]], [[derived-beliefs#Derived Upsert For Invented Nodes]].
- **Requirements added:** [[openspec:datalog-materialization#Materialize a program]], [[openspec:datalog-materialization#Reconcile non-monotonic strata]], 

## Non-goals

- No materialization on commit (DL15 checks constraints; nothing derives automatically).
- No storage of plain-named relations.
- No storage format change.
