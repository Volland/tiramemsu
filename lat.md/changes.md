---
type: Change
---
# Changes

Units of work that moved intent and code together, recorded so decisions can say which change shaped them through `changed_by` edges.

## Tune Datalog Performance

The OpenSpec change of 2026-10-29 that made Datalog closures 2.5–4× faster and `datalog_reason` up to 11×: rank-ordered results, every-instant shortcuts, emit filtering, a row table, resolved accesses and cached plans.

Its decisions R1–R7 are in `openspec/changes/archive/2026-10-29-tune-datalog-performance/design.md`. [[datalog#Rank-Ordered Results]] records R1, and [[datalog#Sorted Columnar Relations]] and [[datalog#Interval Sets Through Joins]] describe the rest as built. Measurements are in [[datalog#Gates And Oracles]] and `bench/datalog/README.md`. Not part of it: sorted columnar relations with leapfrog joins, a thread pool for morsels, and incremental runs ([[datalog#Incremental By DRed]]).

## Add ELPI Engine

The OpenSpec change of 2026-10-31 that added the `tm-elpi` crate: λProlog programs run by lean-elpi 0.4 over a view, with host builtins, structured terms and stated support.

It needed lean-elpi 0.4 (`leanelpi_declare`, `leanelpi_solve_host`, term trees). The facade calls are `View::elpi`, `View::elpi_check` and `View::elpi_conclude`, and the CLI gained `tm elpi query|check|conclude`. Archived in `openspec/changes/archive/2026-10-31-add-elpi-engine/`.

## Add ELPI Programs

The OpenSpec change of 2026-10-31 that added the `tm-elpi-store` crate: `elpi:source` program nodes with includes, `elpi:enforced`, and delta-seeded checks at commit.

The facade gained `TxElpi`, `View::elpi_program_info` and the commit hook beside Datalog's; N3 and Datalog stores refuse nodes that hold ELPI text. Archived in `openspec/changes/archive/2026-10-31-add-elpi-programs/`.

## Add ELPI Materialization

The OpenSpec change of 2026-10-31 that made `Db::materialize_elpi` write ELPI conclusions through the commit shared with N3 and Datalog, reconciling on every run.

The plan is built in `tm-elpi-store`; support is the stated eids plus the program's and its includes' sources. Archived in `openspec/changes/archive/2026-10-31-add-elpi-materialization/`.

## Add ELPI Query Calls

The OpenSpec change of 2026-10-31 that made ELPI goals callable from SPARQL and Cypher, and added the bridge ops and MCP tools.

Goal calls of both languages share one path: `GoalLanguage` on the IR leaf and plan node, and `GoalProviders` in `tm-exec`. Archived in `openspec/changes/archive/2026-10-31-add-elpi-query-calls/`.

