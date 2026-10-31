---
type: Change
---
# Changes

Units of work that moved intent and code together, recorded so decisions can say which change shaped them through `changed_by` edges.

## Tune Datalog Performance

The OpenSpec change of 2026-10-29 that made Datalog closures 2.5–4× faster and `datalog_reason` up to 11×: rank-ordered results, every-instant shortcuts, emit filtering, a row table, resolved accesses and cached plans.

Its decisions R1–R7 are in `openspec/changes/archive/2026-10-29-tune-datalog-performance/design.md`. [[datalog#Rank-Ordered Results]] records R1, and [[datalog#Sorted Columnar Relations]] and [[datalog#Interval Sets Through Joins]] describe the rest as built. Measurements are in [[datalog#Gates And Oracles]] and `bench/datalog/README.md`. Not part of it: sorted columnar relations with leapfrog joins, a thread pool for morsels, and incremental runs ([[datalog#Incremental By DRed]]).
