# A fast Datalog engine with DatalogMTL over valid time

Source: Datalog design interview of 2026-10-28, decisions DL1–DL24 in `lat.md/datalog.md`. This is change 1 of four (DL24): `add-datalog-engine`, then `add-datalog-constraints`, `add-datalog-materialization` and `add-datalog-query-calls`. Prerequisite: tasks 1.1 and 1.2 of `add-interval-algebra` (`Allen`, `Valid::relate`, `Valid::intersects` in `tm-core`).

## Why

N3 reasoning through eyeron ([[reasoning]]) is expressive and gives proofs, but it computes a tuple at a time, keeps one proof per conclusion and, for valid time, re-runs the whole rule set once per segment of the window. On churned agent memory with many valid-time bounds, and for recursive rules such as reporting chains and reachability, that is the slow path. Agents also need to ask goal-directed questions ("who does alice report to, and since when?") and to express integrity constraints over their memory, which N3 does not offer as first-class operations.

A Datalog engine designed for this store can join on ObjectIds over the store's permutations and exact statistics, carry valid-time intervals through its joins instead of sweeping segments, evaluate goals with magic sets, and express temporal rules directly with DatalogMTL's metric operators.

## What Changes

- **New crate `tm-datalog`** on `tm-core` and `tm-store` only (no `exec`, no git dependency, publishable), behind a facade feature `datalog`, off by default. It reads stores only through `ReadStore`, so it runs natively on redb and through `SqlStore` on SQLite.
- **Language:** Soufflé-style text: `@prefix`, rules, `?-` goals, `constraint name warn|error :- body.` Binary atoms `ex:p(S, O)` over stored predicates, `@ E` to bind a statement eid, and reserved atoms `triple/3`, `valid/3`, `in_graph/2`, `added/2` and `tx_instant/2`. Stratified negation (`not`), aggregates (`count`, `sum`, `min`, `max`), comparison, arithmetic, string and date builtins, the 13 Allen relations, and existentials in heads (`exists Y:`).
- **Temporal semantics:** a conclusion holds at an instant iff it follows from the facts valid at that instant. Tuples carry coalesced interval sets over the stored integer-millisecond timeline. DatalogMTL's Horn fragment: sometime and always operators over past and future windows, `since` and `until` in bodies, and the always operators in heads. Negation is the complement within a binding; aggregates are swept along a group's interval endpoints. A program that recurses through a metric operator needs a `window`; one that recurses through value invention needs `limits.maxNewValues`. Transaction time is fixed by the view; `now()` is the view's instant.
- **Evaluation:** semi-naive, set-at-a-time; magic-set rewriting for goals that bind constants; a probe-or-load choice per stored atom from exact statistics; sorted columnar relations with leapfrog and hash joins; parallel morsels except on wasm; provenance only when asked; fuses that report `Incomplete`; budgets and cancellation as everywhere else.
- **Read API:** `View::datalog(program, goal, &DatalogOptions)` returns goal rows, each binding with its valid intervals and an optional proof. `View::datalog_reason` returns every conclusion in the shape of N3's conclusions. `View::program_info` reports strata, monotonicity, metric recursion and predicates used. `View::check` evaluates a program's constraints and returns violations with bindings, eids and intervals.
- **Stored programs:** `TxDatalog::store_program(program, text)` keeps a program as one `(program dl:source "…")` statement; editing is a supersede, so `as_of` uses the program believed then.
- **Every surface:** the JSON bridge (`datalog`, `datalogReason`, `programInfo`, `check`, tx op `storeProgram`), Node, Python, wasm, MCP tools (`datalog_query`, `check_constraints`; `store_program` hidden in read-only mode) and the CLI (`tm datalog`, `tm check`, `tm program put|show`).
- **Benchmarks and oracles:** a `tm-bench` Datalog suite with release gates; differential tests against `tm-n3` on Horn programs and against a brute-force evaluator that checks instant by instant.

## Capabilities

### New Capabilities

- `datalog-language`: the program text, atoms and their mapping to statements, builtins, stratification, safety, termination analysis and the language errors.
- `datalog-evaluation`: goal queries and full reasoning over a view, scope and pruning, magic sets, provenance, limits, budgets, determinism and explain.
- `datalog-temporal`: pointwise valid-time semantics with interval sets, the DatalogMTL operators, Allen relations, windows, negation and aggregates over time, and the pinned transaction clock.
- `datalog-programs`: stored programs, their transaction time, program information and the read surfaces.
- `datalog-constraints`: constraint declarations and on-demand checking. Enforcement at commit follows in `add-datalog-constraints`.

### Modified Capabilities

- `optional-query-frontends`: the facade gains the `datalog` feature, off by default and independent of `exec`.

## Impact

- **New crate `crates/tm-datalog`:** parser, analysis (stratification, safety, termination), magic-set rewriting, the kernel (relations, joins, semi-naive loop, interval sets, temporal operators, aggregates), store access (probe or load), provenance, Allen builtins on `tm_core::Valid::relate`.
- **`crates/tm-core`:** `Valid::relate`, `Valid::intersects` and `Allen` (prerequisite, from `add-interval-algebra`); the `dl:` vocabulary; error variants (`DatalogParse`, `DatalogUnsafe`, `DatalogUnstratifiable`, `DatalogUnboundedRecursion`, `DatalogUnboundedInvention`, `UnknownProgram`, `RuleLanguageConflict`).
- **`crates/tiramemsu`:** feature `datalog`; `View::datalog`, `View::datalog_reason`, `View::program_info`, `View::check`, `TxDatalog::store_program` (`src/datalog.rs`).
- **Bindings, MCP, CLI:** `bindings/json` (feature `datalog`), `bindings/node`, `bindings/python`, `bindings/wasm`, `crates/tiramemsu-mcp`, `crates/tiramemsu-cli`.
- **Bench:** `crates/tm-bench` Datalog suite; Soufflé, Nemo and MeTeoR as external references, run only by the benchmark script.
- **lat.md:** `lat.md/datalog.md` (decisions DL1–DL24, gains `openspec:` frontmatter), `lat.md/architecture.md#Crates`, `lat.md/overview.md#Non-Goals` (DL1 reverses "Datalog as a query language in v1"), `lat.md/bindings.md#JSON Bridge`, `lat.md/api.md`.
- **Binding decisions and constraints:** [[constraints#Read Performance First]], [[constraints#Publishable Crates]], [[constraints#No Unsafe Code]], [[constraints#Same Logical Database]], [[reasoning#Optional Crate]], [[reasoning#Pointwise Valid Time]], [[reasoning#Pinned Clock]], [[reasoning#Stored Rule Sets]], [[backends#Hybrid Join Engine]], [[query#Query Budgets]].
- **Requirements added:** [[openspec:datalog-language#Program text]], [[openspec:datalog-evaluation#Goal queries over a view]], [[openspec:datalog-temporal#Pointwise valid time with intervals]], [[openspec:datalog-programs#Store a program]], [[openspec:datalog-constraints#Check constraints]], [[openspec:optional-query-frontends#Optional Datalog feature]].

## Non-goals

- No writes from a run: materialization is `add-datalog-materialization`.
- No commit-time enforcement: `add-datalog-constraints`.
- No SPARQL or Cypher calls: `add-datalog-query-calls`.
- No operators over transaction time, no well-founded semantics, no restricted chase.
- No replacement of N3: both stay, and N3 serves as an oracle for the Horn subset.
