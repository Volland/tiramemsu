# Enforce Datalog constraints at commit

Source: decisions DL15 and DL16 of `lat.md/datalog.md` (interview of 2026-10-28). Change 2 of four; depends on `add-datalog-engine` (constraint declarations, `View::check`, stored programs).

## Why

`add-datalog-engine` lets an agent ask whether its memory violates a constraint, but nothing stops a write that breaks one: "one CEO at a time", "a task's due date is after its start", "no reporting cycle". The predicate schema enforces a few fixed shapes (`sys:unique`, `sys:one`, value and subject types) inside the writer; constraints written in Datalog generalise that. They must cost in proportion to the write, not the memory, because they run inside the single writer ([[constraints#Single Writer]]).

## What Changes

- **Enforcement as data:** `TxDatalog::enforce(program, on)` writes or retracts `(program dl:enforced true)`, so enforcement is versioned and auditable. Turning it on checks the live data first and fails with `ConstraintViolated` (with all violations) if an `error` constraint is already violated.
- **Commit-time checks:** before every commit, each `error` constraint of each enforced program is evaluated only for bindings that involve the transaction's added statements (positive atoms) or retracted statements (negated atoms), metric windows included. A violation rolls the transaction back with `ConstraintViolated { program, constraint, violations }`. `warn` constraints never block; the commit report lists their new violations.
- **Enforceable shapes:** an `error` constraint may read stored and reserved atoms, non-recursive derived relations (inlined into the body) and metric operators. A constraint over a recursive relation cannot be enforced in this change: `enforce` fails with `ConstraintNotEnforceable` naming it, and it stays check-only.
- **Bounded cost:** checks run under `OpenOptions::constraint_budget` (default 50 ms and 10⁵ seed rows); past it the transaction fails closed with `ConstraintBudgetExceeded`. The commit report gives the constraints checked and the seed rows read.
- **Previews:** a dry-run transaction reports the violations it would cause without failing; `with` speculation fails like a real commit.
- **Both engines:** the check sits in the facade, between the body and the commit, reading the write transaction's own state through `ReadStore` on SQLite and on redb, so both give the same verdict.
- **Surfaces:** bridge tx op `enforce` and the `constraintViolations`/`constraintChecks` fields of transaction reports; Node, Python and wasm; the MCP write tool `enforce_program` (hidden in read-only mode); CLI `tm program enforce|unenforce`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `datalog-constraints` (added by `add-datalog-engine`): enforcement, commit-time checks, budgets, previews and surfaces.

## Impact

- **`crates/tiramemsu`:** `TxDatalog::enforce`, the enforced-program cache on the writer, the pre-commit hook in `Db::transact_leased` (`src/datalog_enforce.rs`), `TxReport` fields, `OpenOptions::constraint_budget`, errors `ConstraintViolated`, `ConstraintNotEnforceable`, `ConstraintBudgetExceeded`.
- **`crates/tm-datalog`:** delta-seeded constraint variants (one per positive atom over added rows, one per negated atom over retracted rows), inlining of non-recursive relations, a seeded-check entry point.
- **Bindings, MCP, CLI** as listed.
- **lat.md:** `lat.md/datalog.md` (DL15, DL16 implemented; a new edge from [[constraints#Single Writer]]), `lat.md/architecture.md#Connections and Concurrency` (work inside the writer), `lat.md/data-model.md#Predicate Schema` (relation to schema checks), `lat.md/bindings.md`, `lat.md/api.md`.
- **Binding decisions and constraints:** [[constraints#Single Writer]], [[reasoning#Explicit Materialization]] (N10 rejected per-commit reasoning; enforcement is a bounded check, not reasoning), [[derived-beliefs#Both Engines One Oracle]], [[query#Query Budgets]].
- **Requirements added:** [[openspec:datalog-constraints#Enforce a program]], [[openspec:datalog-constraints#Error constraints are checked at commit]], [[openspec:datalog-constraints#Checks read only the change]], [[openspec:datalog-constraints#Enforceable constraints]], [[openspec:datalog-constraints#Constraint check budget]], [[openspec:datalog-constraints#Previews report violations]], [[openspec:datalog-constraints#Same verdict on every backend]], [[openspec:datalog-constraints#Constraint surfaces]].

## Non-goals

- No enforcement over recursive relations (needs incremental maintenance of derived relations).
- No automatic repair: a violation fails the write, it never rewrites it.
- No enforcement of N3 rules.
