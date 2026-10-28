## Context

See proposal.md. Every write goes through `Db::transact_leased` (`crates/tiramemsu/src/db.rs`): it takes the writer, runs the body on a `Tx`, checks the budget, and commits through the backend's `transact`. Bulk-import chunks use the same path. Inside the body, `Tx` reads its own uncommitted state through `ReadStore` (`read_store`), and the store offers `added_since(t)` and `retracted_since(t)`, so the transaction's delta is a scan of rows with `t_add = t` or `t_ret = t`. `add-datalog-engine` provides constraint analysis, `View::check` and stored programs.

## Goals / Non-Goals

**Goals:** checks proportional to the transaction; the same verdict on both engines; enforcement versioned as data; a failure that leaves no trace.

**Non-Goals:** recursive constraints, repair, enforcing during `bulk_import` sessions with a different policy (chunks are ordinary transactions and are checked).

## Constraints

- [[constraints#Single Writer]] constrains the hook {strength: must}: work inside the writer delays every write, so the check is seeded, budgeted and skipped when the write touches no predicate a constraint reads.
- [[constraints#Same Logical Database]] constrains the verdict {strength: must}.
- [[derived-beliefs#Both Engines One Oracle]]: the hook sits in the facade above both engines, so neither engine changes.

## Decisions

### DL15a. Pre-commit hook in the facade

After the body returns and before the budget check, `transact_leased` calls `datalog_enforce::check(tx)` when the writer's enforced-program cache is non-empty. It reads the delta (`added_since(t−1)`, `retracted_since(t−1)` restricted to the current transaction) once, groups rows by predicate, and runs only constraints whose read set meets those predicates.

- motivated_by:: [[constraints#Single Writer]] {weight: primary, note: "bounded, delta-seeded, budgeted"}

*Alternatives:* inside both engines (two implementations, D50 parity work); a post-commit check with compensation (violates "a failure leaves no trace").

### DL15b. Delta-seeded variants

For a constraint body `B` (after inlining non-recursive relations), the compiler produces one variant per positive stored atom `A_i` with `A_i` reading the added rows, and one per negated atom `not N_j` with `N_j`'s retracted rows binding its variables and the rest of the body evaluated over the post-state. Metric operators widen the seed's time span by the operator's window, so a dose two hours after an earlier dose is found from either side. Violations are deduplicated by bindings. To ignore pre-existing violations, each violation found is also evaluated on the pre-state (the store before the transaction is not readable inside it, so the pre-state is the post-state minus added rows plus retracted rows, evaluated by the same seeded join); a violation present in both with an equal interval set is not reported.

This is the classical simplification of integrity constraints (Nicolas 1982) over the interval engine.

*Alternatives:* full re-check of each constraint (cost grows with the store).

### DL16. Enforcement statement and cache

`(program dl:enforced true)` is an ordinary statement. The writer keeps a cache of the enforced programs' compiled seeded variants, keyed by the eids of their `dl:source` and `dl:enforced` statements; a transaction that adds or retracts either predicate refreshes it at its end (so enabling enforcement takes effect for the next transaction, while the enabling transaction itself runs the full check of "Enforce a program").

- motivated_by:: [[reasoning#Stored Rule Sets]] {weight: primary}

### Budget

The check installs its own budget (deadline, seed-row cap) from `TxOptions::constraint_budget`, falling back to `OpenOptions::constraint_budget`; the outer transaction budget still applies. Exceeding it returns `ConstraintBudgetExceeded`, which rolls back like any error.

### Dry runs and speculation

`dry_run` collects violations into `TxReport::constraint_violations` and returns the report (whose effects are discarded anyway). `with` uses the ordinary path and fails.

## Risks / Trade-offs

- [Pre-state reconstruction is subtle with supersede] → property test comparing seeded verdicts with full `check` before and after on random histories.
- [Large imports pay per-chunk checks] → seed rows are per chunk; the budget can be raised per transaction; the bulk-import benchmark reports the overhead.
- [A constraint over a hot predicate slows every write] → the report's `constraint_checks` and `seed_rows` and the benchmark's overhead gate (≤ 20 % on a 100-statement transaction at 10⁶) make it visible.

## Migration Plan

Additive. Files without `dl:enforced` statements behave as before; the hook is a cache check. Rollback: unenforce the programs.
