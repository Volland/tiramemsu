## Why

The Datalog closures run 3.1–4.3× slower than Soufflé's interpreter, so the DL20 gate (`[[datalog#Gates And Oracles]]`) is not met. A profile of the benchmark on 2026-10-29 (`sample`, release build, all closure workloads) showed where the time goes:

- About 70 % of `View::datalog` is spent in `goal_rows`. It decodes every cell twice and sorts 1.1 M rows by comparing `Value`s, IRI strings included.
- About 29 % is evaluation, split roughly equally between `merge`/`Rel::insert` and the join.

Most of the gap is in building the result, not in the semi-naive loop.

## What Changes

- **Result path:** decode each distinct term id of a goal relation once. Sort the rows by integer rank keys, computed once from the store's value order. The row order stays exactly as the spec requires.
- **Interval shortcuts:** when a row's set is already every instant (an atemporal run, or `Valid::ALWAYS` statements), skip the interval-set algebra (`subtract`, `union`, `covers`) in inserts and merges.
- **Emit-time filtering:** a derivation whose tuple already holds its whole interval set in the head relation is dropped when emitted, so no `Out` is built for it.
- **Relation storage:** `Rel` finds rows through a hash table of row numbers that hashes into the flat tuple buffer, instead of a map keyed by a copy of each tuple.
- **Hoisted lookups:** each rule variant resolves its stored-atom access paths (loaded predicate and index per bound-position mask) once, instead of taking two locks and cloning two `Arc`s per probe.
- **Plan cache:** plans are cached per rule variant within a stratum. A variant is re-planned when a relation it reads has grown tenfold since it was planned, as DL12 already describes. Explain lines are built once per stratum, not on every iteration of the first one.
- **Phase times:** explain reports the time spent evaluating and the time spent building results. The benchmark prints both, so the gate measures the engine.

Not in this change: sorted columnar relations with merge or leapfrog joins (DL12, task 4.11 of `add-datalog-engine`), a persistent thread pool for morsels, and incremental runs (DL22).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `datalog-evaluation`:
  - [[openspec:datalog-evaluation#Deterministic results]] gains scenarios: the interval shortcuts and the rank-ordered result give the same rows as before.
  - [[openspec:datalog-evaluation#Explain a program run]] gains the evaluation and result times.

## Impact

- Code: `crates/tm-datalog/src/{run.rs, rel.rs, interval.rs, eval.rs, source.rs}`, `crates/tm-bench/examples/datalog.rs`, and `bench/datalog/README.md` (re-measured).
- Dependencies: `tm-datalog` adds `hashbrown`, already in the lock file through other crates, for `HashTable`.
- APIs: no signature changes. Explain output gains two lines.
- Docs touched:
  - `lat.md/datalog.md`: [[datalog#Sorted Columnar Relations]], [[datalog#Gates And Oracles]] (new measurements) and [[datalog#Interval Sets Through Joins]].
  - `lat.md/tests.md`: Datalog test specs for the new scenarios.
- Binding edges:
  - [[datalog#Gates And Oracles]] and [[datalog#Probe Or Load Per Atom]] are `motivated_by` [[constraints#Read Performance First]].
  - The open `-contradicts` edge from `Rel` to [[datalog#Sorted Columnar Relations]] stays open, because this change keeps hash indexes.
  - [[openspec:datalog-evaluation#Deterministic results]] and [[datalog#Provenance On Demand]] (tracked and untracked runs return the same tuples) constrain every optimisation here.
