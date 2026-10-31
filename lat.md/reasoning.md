---
type: Decision
openspec: [n3-reasoning, n3-rule-sets, n3-materialization]
---
# Reasoning

Decisions N1–N18 of the N3 reasoning interview of 2026-10-28: Notation3 rules run by eyeron over a view, aware of both clocks, with conclusions materialized as derived beliefs.

The work is three OpenSpec changes: `add-derived-beliefs` (the prerequisite: `derive`, support links, the support cascade), `add-n3-reasoning` (the crate `tm-n3`, read-only reasoning, rule sets and materialization) and `publish-eyeron-dependency` (moving eyeron from a git pin to crates.io). The site's [N3 book](../site/n3/index.html) explains it for users.

## Read-Only And Materialized

N1: `View::reason` answers "what follows" without writing; `Db::materialize` writes conclusions as derived beliefs so they fall when their grounds go. Both ship together; materialization needs `derive`.

- motivated_by:: [[constraints#Never Forget]] {weight: primary, since: "2026-10-28"}

Rejected: materialize only (no way to ask without writing) and read-only only (conclusions never become memory and never benefit from the support cascade in [[time-model#Cascade]]).

## Eyeron From Crates.io

N2: eyeron comes from crates.io (0.8.25) with `default-features = false`, so its `ureq` client is left out and `tm-n3` is publishable.

- motivated_by:: [[constraints#Publishable Crates]] {weight: primary, since: "2026-10-28"}

Until eyeron was published the workspace pinned a git commit, and Cargo refuses to publish a crate with any git dependency, optional ones included. Published 0.8.25 has the `cli` feature gating `ureq`; `default-features = false` matters because the 0.9.0 linux-aarch64 wheel failed to cross-compile `ring`, which only eyeron's HTTP client pulled in. The published release has no `ReasonerOptions::now`, so the `time:localTime` rewrite stays. Vendoring was rejected: it would make this project the maintainer of a fast-moving reasoner.

## Optional Crate

N3: `tm-n3` depends only on `tm-core` and eyeron; the facade feature `n3` is off by default and independent of `exec`, so the core tier can reason.

- motivated_by:: [[constraints#Read Performance First]] {weight: secondary, since: "2026-10-28"}

`tm-n3` never reads or writes the store: the facade loads statements through the view and writes through its `Tx`, which keeps the crate engine-agnostic across the SQL and trait engines ([[backends#One Engine In Two Phases]]).

## Scope And Pruning

N4: a run loads one scope (whole view, named graphs, or seeds within a number of hops) and, inside it, only statements whose predicate some rule body uses; a variable predicate turns pruning off.

- motivated_by:: [[constraints#Read Performance First]] {weight: primary, since: "2026-10-28"}

A variable predicate inside the object formula of `tm:content` does not turn pruning off: those formulas describe statements already loaded, and a rule set that reads `tm:content`, `tm:validFrom` or `tm:validTo` also loads, by eid, the statements its loaded statements are about. The loader is [[crates/tiramemsu/src/n3.rs]] and the analysis [[crates/tm-n3/src/rules.rs]].

Graph scopes reuse [[data-model#Named Graphs]], so one agent's or one session's beliefs can be reasoned over alone, as in [[recipes#Reasoning Inside One Graph]]. Lazy loading through a callback from eyeron is deferred until the benchmark shows loading dominates.

## Pointwise Valid Time

N5: a conclusion holds at an instant exactly when it follows from the statements valid at that instant; runs sweep the segments between valid-time bounds and merge equal neighbours.

- motivated_by:: [[constraints#Never Forget]] {weight: secondary, since: "2026-10-28"}

This is [[time-model#Valid Time]] applied to inference: two facts that were never valid together never combine. Intersecting premise intervals after one global run was rejected as unsound under negation and incomplete, because eyeron keeps one proof per conclusion. A `valid_at` view is a sweep of one segment; `max_segments` bounds the sweep and reports `Incomplete`, like [[query#Physical Planning#Path Engine#Path Completeness]].

## Statements As IRIs

N6: a statement enters N3 as `urn:tiramemsu:stmt:N`, with virtual `tm:content`, `tm:validFrom` and `tm:validTo` triples generated when a rule uses them; nodes are `urn:tiramemsu:node:N`.

- motivated_by:: [[constraints#Same Logical Database]] {weight: secondary, since: "2026-10-28"}

Identity stays the eid, so episodes and parallel derivations stay distinct and premises map back to eids for support. Quoted formulas alone would collapse statements with equal content.

## Pinned Clock

N7: `now` is the view's transaction-time instant, fixed for the whole run; `time:localTime` returns it, so as-of reasoning is reproducible.

- motivated_by:: [[constraints#Never Forget]] {weight: secondary, since: "2026-10-28"}

Each segment also offers its bounds as `tm:run tm:segmentFrom` and `tm:segmentTo`, keeping the two time axes apart. Until eyeron accepts a `now` option, `tm-n3` rewrites `time:localTime` premises.

## Stored Rule Sets

N8: a rule set is a node with one `n3:source` statement holding its N3 text; editing it is a supersede, so `as_of` reasons with the rules believed then.

- motivated_by:: [[constraints#Never Forget]] {weight: primary, since: "2026-10-28"}

`tm:` stays reserved for virtual predicates, so stored rule vocabulary uses `n3:` = `urn:tiramemsu:n3:`.

## Rules Are Support

N9: a materialized conclusion's support is its premises plus the rule-set statement, so editing or retracting the rules retracts what they concluded; `n3:derivedBy` and `n3:rule` are labels.

- motivated_by:: [[reasoning#Stored Rule Sets]] {weight: primary, since: "2026-10-28"}

This reuses the support cascade of [[time-model#Cascade]]; `View::dependents` of the rule-set statement previews a rule edit.

## Explicit Materialization

N10: materialization runs only when called, and a run excludes its own rule set's earlier conclusions from its input; other rule sets' conclusions are ordinary facts.

- motivated_by:: [[constraints#Single Writer]] {weight: primary, since: "2026-10-28"}

Reasoning on every commit was rejected because it would put a reasoner inside the single writer for every write.

## Monotonic Materialization

N11: rule sets that use negation as failure, aggregates, the clock or `list:map` are refused by `materialize`; `reason` accepts them. A fired inference fuse aborts `materialize`.

- motivated_by:: [[constraints#Never Forget]] {weight: primary, since: "2026-10-28"}

Adding a fact can invalidate a non-monotonic conclusion, and the cascade only reacts to retractions, so stored conclusions would stop being grounded. `list:map` is monotonic, but eyeron's proof records only the builtin triple, not the facts it read, so its conclusions would be stored with incomplete support (class `UntracedPremises`). Reconciliation by diffing runs is an open follow-up for N3; Datalog programs have it ([[datalog#Shared Materialization With Reconciliation]]).

## Merge By Support

N12: adjacent segments merge only when the conclusion and its support are identical; otherwise they become separate episodes, each falling with its own support.

- motivated_by:: [[reasoning#Rules Are Support]] {weight: primary, since: "2026-10-28"}

eyeron returns one proof per conclusion and segment, so a stored conclusion may rest on one of several possible supports; it is sound, and the next `materialize` restores it if that support goes.

## Snapshot Then Commit

N13: `materialize` reasons on a read view outside the writer and writes in one short transaction; conclusions whose support was retracted meanwhile are skipped as `stale_support`.

- motivated_by:: [[constraints#Single Writer]] {weight: primary, since: "2026-10-28"}

Support liveness, checked by `derive`, makes the race harmless: nothing is written whose grounds are gone, and missed conclusions come back on the next run. The snapshot and the commit are in [[crates/tiramemsu/src/n3_materialize.rs]]. Conclusions the store cannot hold (a list or formula term, a literal subject, a reserved or non-IRI predicate) are skipped with their dependents and reported as `unstorable`; other write errors, such as schema checks, abort the run.

## Skolem Nodes

N14: a blank node in a conclusion becomes a node found or created by `upsert_derived` with a deterministic key, derived from the rule firing's support.

- motivated_by:: [[reasoning#Explicit Materialization]] {weight: primary, since: "2026-10-28"}

Re-runs reuse the node, so `derive` stays idempotent; when the support goes, the key goes too and the node is left unused, never deleted.

## Structured Proofs

N15: every conclusion carries its rule and premise eids; N3 proof text is opt-in; materialized proofs are the support links, never stored text.

- motivated_by:: [[reasoning#Rules Are Support]] {weight: secondary, since: "2026-10-28"}

## Widening

N16: when a run finds a stored derivation with the same content and support over a longer interval, it supersedes it to the hull; intervals grow, never shrink.

- motivated_by:: [[constraints#Never Forget]] {weight: primary, since: "2026-10-28"}

A conclusion that stops holding loses support and is retracted, not narrowed. `require_complete` refuses to commit a run that hit a limit (`N3Incomplete`). A statement written earlier in the same run is never widened, since superseding it would retract what the run built on it; the uncovered part becomes a separate, touching statement.

## Every Surface

N17: Rust, the JSON bridge, Node, Python, wasm, MCP and the CLI expose `reason`, `store_rules` and `materialize`; SPARQL and Cypher only read materialized statements.

- motivated_by:: [[reasoning#Optional Crate]] {weight: primary, since: "2026-10-28"}

A reasoner's input is a whole scope, not a row, so it does not fit as a function inside a query. The bridge calls are in [[bindings#JSON Bridge#N3 Reasoning]]; the MCP tools and `tm` commands in [[api#MCP Tools]] and [[api#CLI]].

## Tests Prove Semantics

N18: conformance against eyeron's examples, a property test that sweeps agree with snapshots, a grounding and convergence property test, scenario tests and a benchmark on both backends.

- motivated_by:: [[reasoning#Pointwise Valid Time]] {weight: primary, since: "2026-10-28"}

The benchmark (`tm-bench`, `examples/reason.rs`, feature `n3`) measured on 2026-10-28 at 10 000 statements: a pruned two-hop join loading 499 statements took about 8 ms with one segment and 18 ms with 17, the same rule atemporal 8.5 ms, and an unpruned load of the whole store 73 ms; SQLite and redb were within a few percent.
