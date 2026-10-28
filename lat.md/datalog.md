---
type: Decision
openspec: [datalog-language, datalog-evaluation, datalog-temporal, datalog-programs, datalog-constraints, datalog-materialization]
---
# Datalog

Decisions DL1–DL24 of the Datalog interview of 2026-10-28: a fast Datalog in its own crate, with DatalogMTL over valid time, for queries, reasoning and constraints beside N3.

The four OpenSpec changes of [[datalog#Four Changes]] were written on 2026-10-28. `add-datalog-engine` is implemented in [[crates/tm-datalog/src/lib.rs]] and the facade's [[crates/tiramemsu/src/datalog.rs]] (DL1–DL12, DL16 for storage, DL18–DL21, DL23, DL25, DL26); `add-datalog-constraints` implements DL15 and DL16's enforcement ([[crates/tm-datalog/src/enforce.rs]], [[crates/tiramemsu/src/datalog_enforce.rs]]); `add-datalog-materialization` implements DL13, DL14 and DL21's storage ([[crates/tiramemsu/src/datalog_materialize.rs]], [[crates/tiramemsu/src/materialize.rs]]); DL22 (incremental runs) is not built; `add-datalog-query-calls` follows. N3 stays ([[reasoning]]): it keeps quoted formulas and proof text, and Datalog is the set-at-a-time path to the same derived beliefs.

## One Language Three Uses

DL1: Datalog serves goal queries (`View::datalog`), reasoning and materialization as N3 does, and integrity constraints. This reverses the v1 non-goal "Datalog as a query language" in [[overview#Non-Goals]].

- motivated_by:: [[constraints#Read Performance First]] {weight: primary, since: "2026-10-28"}

Rejected: reasoner and constraints only (goal queries would lose magic sets) and a query language only (no alternative to N3 reasoning).

## Own Kernel Over ReadStore

DL2: `tm-datalog` depends only on `tm-core` and `tm-store`, not on `exec`, and has its own semi-naive join kernel; it runs natively on redb and through `SqlStore` on SQLite.

- motivated_by:: [[reasoning#Optional Crate]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[constraints#Publishable Crates]] {weight: secondary, since: "2026-10-28"}

A recursive program spends its time joining in-memory relations, and the native runner's `Bgp` carries SPARQL and Cypher semantics (canonical eids, isomorphism, three-valued logic) that the hot loop does not need. With no git dependency the crate is publishable, unlike `tm-n3`. Rejected: reusing `Bgp` (needs `exec`) and extracting a shared join crate first (a refactor of tested code before any Datalog exists).

## Souffle-Style Text

DL3: programs are Soufflé-style text with `@prefix`, rules, `?-` goals and `constraint name warn|error :- body.`; this one form is stored, parsed and passed through every binding.

- motivated_by:: [[datalog#One Language Three Uses]] {weight: primary, since: "2026-10-28"}

Rejected: Datomic-style data syntax (verbose for recursion) and text plus a JSON AST (two front ends to keep equal).

## Binary And Reserved Atoms

DL4: `ex:p(S, O)` reads a stored predicate under set semantics in the view, `ex:p(S, O) @ E` also binds the eid, and reserved atoms `triple`, `valid`, `in_graph`, `added` and `tx_instant` reach the rest.

- motivated_by:: [[reasoning#Statements As IRIs]] {weight: secondary, since: "2026-10-28"}

Statement eids are terms, so layers are ordinary atoms such as `ex:confidence(E, C)`. `triple(S, P, O)` with a variable predicate turns predicate pruning off, as in [[reasoning#Scope And Pruning]]. Rejected: one generic quad atom (defeats per-predicate statistics) and eids on every atom (bag semantics everywhere).

## Interval Sets Through Joins

DL5: a conclusion holds at an instant exactly when it follows from the facts valid then; tuples carry coalesced interval sets, joins intersect them and unions merge them, so one pass replaces the segment sweep.

- motivated_by:: [[reasoning#Pointwise Valid Time]] {weight: primary, since: "2026-10-28"}

This is N5's meaning computed differently: N3 sweeps segments because eyeron keeps one proof per conclusion, while this engine sees every derivation of a stratum. Results equal N3's on the same Horn rules ([[datalog#Gates And Oracles]]). `time: atemporal` stays available. Negation and aggregates are [[datalog#Stratified Negation In The Interval Engine]].

## DatalogMTL Horn Fragment

DL6: v1 has DatalogMTL's metric operators in bodies (sometime and always, past and future, `since`, `until`), the always operators in heads, and the 13 Allen relations with interval arithmetic as builtins.

- motivated_by:: [[datalog#Interval Sets Through Joins]] {weight: primary, since: "2026-10-28"}

Keywords: `past[0,3d]`, `always_past[a,b]`, `future[a,b]`, `always_future[a,b]`, `A since[a,b] B`, `A until[a,b] B`. A sometime operator in a head is not Horn and is refused. The Allen builtins use `tm_core::Valid::relate`, built first as task 1 of `add-interval-algebra`, so SPARQL and Cypher share it. Recommended was Allen and metric past first with the rest later; the full fragment was chosen.

## Integer Timeline

DL7: the timeline is the stored one, integer epoch milliseconds with half-open intervals and symbolic open ends; only a program that recurses through a metric operator needs an explicit `window`.

- motivated_by:: [[datalog#DatalogMTL Horn Fragment]] {weight: primary, since: "2026-10-28"}

Static analysis finds dependency cycles through a metric operator; without a `window` such a program fails with `DatalogUnboundedRecursion`. Within a window the coalesced fixpoint is finite. A fuse (iterations, interval fragments) reports `Incomplete` as N3's limits do. Rejected: a dense rational timeline (endpoint conversion, open and closed bounds) and a window for every metric program.

## Stratified Negation In The Interval Engine

DL8: negation and aggregates are stratified and pointwise: `not p(X)` is the complement of p's interval set for a binding bound positively, and an aggregate sweeps the interval endpoints of its group.

- motivated_by:: [[datalog#Interval Sets Through Joins]] {weight: primary, since: "2026-10-28"}

Programs that cannot be stratified are rejected with the cycle named. No stratum is re-run per global segment, so cost follows the endpoints a group touches, not the segment count of the window. Rejected: the whole-program sweep of N3 and well-founded semantics.

## Pinned Transaction Time

DL9: a run reads one view; `now()` is its transaction instant, fixed for the run; operators range over valid time only, and transaction time is data through `added` and `tx_instant`.

- motivated_by:: [[reasoning#Pinned Clock]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[constraints#Grounded Memory]] {weight: secondary, since: "2026-10-28"}

As-of runs are reproducible. Rejected: bitemporal operators (rectangles in every join) and reasoning over history views (conflicts with grounded materialization).

## Magic Sets For Goals

DL10: goals are evaluated bottom-up and semi-naively, and a goal that binds constants is rewritten with demand (magic-set) predicates, falling back per stratum to the full program where the rewrite cannot apply.

- motivated_by:: [[constraints#Read Performance First]] {weight: primary, since: "2026-10-28"}

The fallback applies when the rewrite would break stratification or a metric operator blocks binding propagation, and explain names it. A property test checks that rewritten and full programs return the same rows. Rejected: top-down tabling (a second, tuple-at-a-time engine) and full evaluation only.

## Probe Or Load Per Atom

DL11: for each stored atom the planner either probes the store's permutations or loads the matching statements once into an in-memory index for the binding pattern it needs, from exact statistics and the expected probe count.

- motivated_by:: [[constraints#Read Performance First]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[reasoning#Scope And Pruning]] {weight: secondary, since: "2026-10-28"}

Selective atoms in non-recursive rules and magic-set goals probe; atoms a recursive stratum reads every iteration load. Only predicates some body uses are read, within an optional scope of graphs or seeds plus hops. Explain shows each atom's choice. Rejected: always loading (as Soufflé and Nemo do) and always probing.

## Sorted Columnar Relations

DL12: relations are flat `i64` columns sorted once per needed index, with interval sets as a parallel column; joins are leapfrog triejoin for cyclic or multiway bodies and hash joins for binary acyclic steps.

- motivated_by:: [[backends#Hybrid Join Engine]] {weight: primary, since: "2026-10-28"}

As built, relations are tuples in one flat buffer with hash indexes per bound-position mask, built on first use and kept current on insert, joined by index nested loops ([[crates/tm-datalog/src/rel.rs]], [[crates/tm-datalog/src/eval.rs]]); the worst-case-optimal join for cyclic bodies is not built yet and is recorded as debt on the code. Indexes are chosen per binding pattern, as Soufflé does. Delta and full relations merge by sort-merge with coalescing; each rule variant is ordered from actual sizes and re-planned when a delta changes size tenfold; the delta is cut into parallel morsels. Only ObjectIds are joined and terms are decoded at the output. Rejected: hash tables only (cyclic blow-up, no ordered sweeps) and B-tree relations.

## Shared Materialization With Reconciliation

DL13: `materialize_program` shares N3's path (support with the program statement, one derivation per stored fragment, snapshot then commit, widening); strata with negation or aggregates also materialize, by reconciliation.

- motivated_by:: [[reasoning#Rules Are Support]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[reasoning#Snapshot Then Commit]] {weight: primary, since: "2026-10-28"}

The common parts moved out of [[crates/tiramemsu/src/n3_materialize.rs]] into [[crates/tiramemsu/src/materialize.rs]], which both languages use; N3's tests ran unchanged. Reconciliation diffs a run against the program's live conclusions and retracts those no longer derived; the report marks such strata `needs_rerun_after_writes` (the predicates they conclude), because a later write can invalidate them before the next run. This closes N11's open follow-up for Datalog ([[reasoning#Monotonic Materialization]]). Rejected: refusing non-monotonic strata as N3 does, and storing every derivation.

## Reconciliation Is An Explicit Retraction

DL14: a reconciliation retracts through `Tx::retract`, so `ret_kind` is 0 and the cascade runs; the reified materialize transaction carries `dl:reconciled` with the program and a count.

- motivated_by:: [[constraints#Never Forget]] {weight: primary, since: "2026-10-28"}

No storage format change: `RetKind::from_i64` knows 0 to 3 only, so a new kind would lock older binaries out of newer files. As built, a run that retracts anything also writes `(tx dl:reconciledCount n)`, and every run writes `(tx dl:materialized program)`.

## Warn And Error Constraints

DL15: `warn` constraints are check-only through `View::check`; `error` constraints run at commit inside the writer, seeded only by the transaction's added and retracted statements, under a budget that fails closed.

- motivated_by:: [[constraints#Single Writer]] {weight: primary, since: "2026-10-28", note: "bounded, delta-seeded and budgeted, unlike the per-commit reasoning N10 rejected"}

A violation reports bindings, eids and intervals, at any as-of or valid-at view. An `error` constraint may read stored atoms, inlined non-recursive rules and metric operators; one over recursive rules is check-only in v1. Registering an `error` constraint that live data violates is rejected with the violations, as schema changes are ([[data-model#Predicate Schema]]). A view over the writer reads uncommitted state through `ReadStore`, and `added_since` and `retracted_since` give the delta, so the check sits in the facade and behaves the same on both engines. `with` speculation previews violations. Over budget, the transaction fails with `ConstraintBudgetExceeded`. As built, a constraint body is unfolded through its non-recursive derived atoms; each positive stored atom gets a variant reading only the added rows whose constants fit it, and each negated one a variant starting from the retracted rows; a violation is kept when the view as of the previous transaction did not already hold it over the same instants. A program whose text or enforcement changed is checked in full. Aggregates, `since`/`until` and operators with an infinite window make an `error` constraint not enforceable.

## Program Nodes

DL16: a program is a node with one `dl:source` statement holding its text, `dl:` being `urn:tiramemsu:dl:`; editing is a supersede, and `(program dl:enforced true)` turns its `error` constraints on.

- motivated_by:: [[reasoning#Stored Rule Sets]] {weight: primary, since: "2026-10-28"}

As-of runs and checks use the program believed then, and enforcement is versioned like any statement. As built there is no cache: each commit reads the `dl:enforced` marks and the program text from the transaction's own state, which costs one dictionary lookup when nothing is enforced and cannot go stale. A node holding both N3 rules and a program fails with `RuleLanguageConflict`.

## Goal Calls In SPARQL And Cypher

DL17: Cypher `CALL tiramemsu.datalog.query(program, goal)` and a SPARQL `SERVICE <urn:tiramemsu:dl:query>` run one goal with constant arguments once per query, never per row, under the query's budget.

- motivated_by:: [[datalog#Magic Sets For Goals]] {weight: primary, since: "2026-10-28"}

This follows text recall's table-call pattern: a `tm_datalog` table function on SQLite and a native operator on redb, behind facade features `exec` and `datalog`. It differs from [[reasoning#Every Surface]], where a whole-scope reasoner fits no query; a bound goal is a table. Rejected: separate entry points only, and derived predicates as virtual predicates (per-row calls defeat magic sets).

## Own Calls On Every Surface

DL18: Datalog has its own calls, not a language flag on N3's, behind a facade feature `datalog` that is off by default, and every binding, the MCP server and the CLI build with it.

- motivated_by:: [[reasoning#Every Surface]] {weight: primary, since: "2026-10-28"}

Rust has `View::datalog`, `View::datalog_reason` (conclusions shaped as N3's), `View::check`, `View::program_info`, `TxDatalog::store_program`, `TxDatalog::enforce` and `Db::materialize_program`. The JSON bridge ops `datalog`, `datalogReason`, `check`, `programInfo`, `storeProgram`, `enforce` and `materializeProgram` reuse N3's `scope`, `window`, `limits`, `budget`, `requireComplete` and `dryRun` shapes and fail with `Unsupported` without the feature. Node, Python, wasm, the MCP server (`datalog_query`, `check_constraints`, write tools hidden in read-only mode) and the CLI (`tm datalog`, `tm check`) build with it. Rejected: a `language` parameter on the N3 calls.

## Provenance On Demand

DL19: queries and checks run without provenance; it turns on for `materialize`, for `error` violations and for `proof: true`, keeping one derivation per tuple fragment in a side array.

- motivated_by:: [[reasoning#Merge By Support]] {weight: primary, since: "2026-10-28"}

A derivation is a rule id with references to premise tuples, resolved to eids only at the end. Tracked and untracked runs return the same tuples, which a property test checks. Rejected: always tracking (double memory) and full why-provenance.

## Gates And Oracles

DL20: a `tm-bench` Datalog suite on both backends gates the release, and three oracles check results: `tm-n3` on Horn programs, a brute-force pointwise evaluator, and MeTeoR on temporal sets.

- motivated_by:: [[reasoning#Tests Prove Semantics]] {weight: primary, since: "2026-10-28"}

Workloads at 10⁵ and 10⁶ statements: transitive closure (chain, hub, small world), same-generation, an RDFS and OWL 2 RL subset on WatDiv, DatalogMTL on a temporal LUBM-style set, and constraint-check cost against transaction size; references are Soufflé's interpreter, Nemo, MeTeoR and eyeron. Proposed gates: ten times `tm-n3` on the same Horn programs, within twice Soufflé's interpreter on closure and same-generation, and at most 20 % added to a 100-statement transaction at 10⁶ by enforced constraints. The brute-force evaluator checks DatalogMTL property tests instant by instant. Measured on 2026-10-28 (`bench/datalog/README.md`): 6.6× `tm-n3` on a hub closure (N3 did not finish the small-world closure in 491 s) and 3.1–4.3× slower than Soufflé's interpreter on closures, so neither gate is met yet; demand goals run in milliseconds. Tuning is open.

## Guarded Value Invention

DL21: a head may invent a node with `exists Y`, made a skolem node through `upsert_derived`, and compute values with arithmetic, string and date builtins; recursion through invention needs `limits.maxNewValues`.

- motivated_by:: [[reasoning#Skolem Nodes]] {weight: primary, since: "2026-10-28"}

The skolem key is the rule and its frontier bindings, so re-runs reuse the node. Without the limit such a program fails with `DatalogUnboundedInvention`; with it, the fuse reports `Incomplete`. Programs without recursion through invention always terminate. Rejected: pure Datalog and a restricted chase.

## Incremental By DRed

DL22: the engine keeps no state between runs; a materialize transaction carries `(tx dl:materialized program)`, and the next run seeds monotonic strata from `added_since` and re-derives what the cascade retracted since then.

- motivated_by:: [[datalog#Shared Materialization With Reconciliation]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[reasoning#Merge By Support]] {weight: secondary, since: "2026-10-28"}

The record is a transaction label rather than a statement on the program, so runs do not supersede a statement each time (refined while writing `add-datalog-materialization`). The program's live conclusions are loaded as the full relation. A conclusion retracted through its one stored support is re-checked with a goal query, DRed's rederive step. Non-monotonic strata recompute and reconcile, `full: true` forces a full run, and a property test checks incremental against full.

Not built. `materialize_program` evaluates the whole program on a snapshot on every call and writes only the difference, so a re-run asserts nothing but the label; evaluating from a delta needs the engine to seed semi-naive from added statements over existing conclusions, which is a change of its own. The label of DL14 is written so that such a run has its base. Rejected: always recomputing and a warm in-memory engine updated on each commit.

## Sequential On Wasm

DL23: morsel parallelism sits behind a cargo feature; `wasm32-unknown-unknown` has no threads and runs sequentially, with identical results.

- motivated_by:: [[datalog#Sorted Columnar Relations]] {weight: primary, since: "2026-10-28"}

## Four Changes

DL24: the work is a prerequisite and four OpenSpec changes, with enforcement before materialization: engine, constraints, materialization, query calls.

- motivated_by:: [[datalog#One Language Three Uses]] {weight: primary, since: "2026-10-28"}

0. `Valid::relate` in `tm-core`, task 1 of `add-interval-algebra`.
1. `add-datalog-engine`: the crate (parser, stratification, kernel, probe or load, magic sets, the interval engine with DatalogMTL and Allen, invention, fuses), the read and store ops, `warn` checks, every binding, MCP, the CLI, the benchmark suite and the oracles.
2. `add-datalog-constraints`: `error` enforcement at commit, `dl:enforced` and the writer cache.
3. `add-datalog-materialization`: the shared materialize module, reconciliation and DRed.
4. `add-datalog-query-calls`: the SPARQL `SERVICE` and Cypher `CALL` through `tm_datalog` and a native operator.

Rejected: materialization before constraints, and one change.

## Statement Records Hold Always

DL25: `stmt(E, S, P, O)` is a statement's record and holds at every instant, while `p(S, O)`, `p(S, O) @ E` and `triple` are facts that hold over the statement's valid time.

- motivated_by:: [[datalog#Interval Sets Through Joins]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[datalog#Binary And Reserved Atoms]] {weight: secondary, since: "2026-10-28"}

Found while testing Allen relations: under pointwise semantics two facts that were never valid together never meet in one body, so "job x meets job y" needs the records. `valid`, `added` and `tx_instant` are records too.

## IRI Heads Hold Stored Statements

DL26: a predicate IRI that a head defines is a derived relation that also holds the stored statements of that predicate, as in N3, so stored and derived facts combine and only the derived part is a conclusion.

- motivated_by:: [[datalog#Binary And Reserved Atoms]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[datalog#Gates And Oracles]] {weight: secondary, since: "2026-10-28", note: "the N3 differential needs the same meaning"}

Each IRI head gets an implicit seed rule from its stored statements. A rule that negates its own head predicate is therefore recursion through negation and is refused, so a rule inventing a manager for the unmanaged derives a differently named predicate.

