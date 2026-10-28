## Context

Tiramemsu is bitemporal ([[time-model#Transaction Time]], [[time-model#Valid Time]]) and never deletes. Statements are first-class: a statement's eid can be the subject or object of another statement (layers). `add-derived-beliefs` adds `Tx::derive(s, p, o, valid, support)`, a support cascade that retracts a derived statement when any support is retracted, `upsert_derived` and widening by supersede.

eyeron (MIT, Jos De Roo, <https://github.com/eyereasoner/eyeron>, 0.8.22 at commit `c4adb8c`) is an in-memory N3 reasoner: `reason(&Document, &ReasonerOptions) -> ReasonerResult` with `derived`, `proofs: Vec<DerivedFact { fact, rule, premises, bindings }>`, `status`, `limits_reached` and `fuse` (a fired `=> false` rule). Its terms are `Iri`, `Var`, `Blank`, `Literal`, `List` and `Formula` (no RDF 1.2 triple terms). Its library does no I/O, but its manifest depends on `ureq` unconditionally for its binaries, it is not on crates.io, and `time:localTime` reads the system clock.

There are two write engines (SQL in `tm-core`, trait in `tm-store`); `tm-n3` never writes itself, so it is engine-agnostic: the facade loads facts through `View::triples` and writes through the facade `Tx`.

## Goals / Non-Goals

**Goals:** reasoning that never combines facts that were not simultaneously valid; reproducible as-of reasoning; materialized conclusions that are always grounded; an optional, off-by-default dependency.

**Non-Goals:** incremental or automatic reasoning; non-monotonic materialization; reasoning inside SPARQL or Cypher.

## Decisions

### N1. Read-only first, materialization on derived beliefs

`View::reason` returns conclusions without writing; `Db::materialize` writes them through `derive`. Both ship in this change; materialization requires `add-derived-beliefs`.

- depends_on:: [[openspec:derived-beliefs#Derive a belief with support]]

*Alternatives:* materialize only (no way to ask "what follows" without writing); read-only only (conclusions never become memory and never benefit from the cascade).

### N2. eyeron through crates.io, git pin meanwhile

The upstream crate gains a `cli` feature that gates `ureq` and the binaries, and `ReasonerOptions::now`; it is then published. Until then the workspace pins eyeron to a git commit and `tm-n3` is `publish = false` (change `publish-eyeron-dependency`). Because Cargo refuses to publish a crate with any git dependency, optional or not, the facade's `n3` feature blocks publishing `tiramemsu` until then; this is a release constraint.

*Alternatives:* vendoring 16k fast-moving lines (we would maintain someone else's reasoner); git pin forever (never publishable).

### N3. A separate crate behind an off-by-default facade feature

`tm-n3` depends on `tm-core` (values, eids, the RDF rendering) and `eyeron`, never on `tm-exec`; the facade's `n3 = ["dep:tm-n3"]` exposes the API. This is the pattern of `tm-sparql` and `tm-cypher` ([[architecture#Crates#Cargo Features]]).

*Alternatives:* a `tiramemsu-n3` extension crate over the facade (bindings would depend on two crates kept in lockstep); on by default (pulls `regex` and `num-bigint` into every build).

### N4. Scope plus predicate pruning

A run loads one scope of the view: everything (default), the members of named graphs, or the statements within `hops` of seed nodes. Within it, only statements whose predicate appears in a predicate position anywhere in a rule body (forward or backward rules, including nested formulas) are loaded. A variable in a predicate position turns pruning off for the run. The result reports the number of loaded statements and whether pruning applied.

*Alternatives:* load the whole view (cost proportional to the store); lazy pattern callbacks from eyeron (needs a fact-source trait upstream; deferred until measured).

### N5. Pointwise valid time

A conclusion holds at `t` iff it follows from the statements of the scope that are valid at `t`. A `valid_at(t)` view is one snapshot. Otherwise the run takes the window (default the whole timeline), cuts it at every `v_from` and `v_to` of the loaded statements inside it, runs eyeron once per segment on the statements valid throughout it, and merges adjacent segments whose conclusion **and support** are identical (N12). `max_segments` (default 1 024) bounds the sweep; past it the run stops and reports `Incomplete { limits: [Segments] }`. `TimeMode::Atemporal` is the explicit opt-out that ignores valid time.

- motivated_by:: [[time-model#Valid Time]] {weight: primary}

*Alternatives:* intersecting premise intervals after one run over everything (unsound under negation and incomplete because eyeron keeps one proof per fact); snapshot only (cannot say when a conclusion held).

### N6. Statements as their IRIs with content

Values map through the RDF rendering the bundle export uses ([[crates/tm-core/src/rdf.rs#render]]): IRIs, literals, and `urn:tiramemsu:node:N`, `bnode:N`, `stmt:N` for anonymous nodes, blank nodes and statements. For each loaded statement `e`, the virtual triples `e tm:content { s p o }`, `e tm:validFrom dt` and `e tm:validTo dt` (bounded ends only, `xsd:dateTime`) are generated when a rule body mentions that predicate or has a variable predicate; they are never stored. Identity is the eid, so episodes and parallel derivations stay distinct and premises map back to eids.

*Alternatives:* quoted formulas only (content-identified, so episodes collapse and premises cannot be traced to eids).

### N7. The clock is the view's instant

`now` is the view's transaction-time instant (the as-of instant, or one instant captured when a run on the now view starts), fixed for the whole run and reported. `time:localTime` yields it as `xsd:dateTime`, and `tm:run tm:now dt` is available to rules. During a sweep each segment also offers `tm:run tm:segmentFrom dt` and `tm:segmentTo dt` (bounded ends). Until eyeron has `ReasonerOptions::now`, `tm-n3` rewrites `time:localTime` premises to `tm:run tm:now ?o`.

*Alternatives:* wall clock (irreproducible; as-of runs conclude about the present); the segment start as `now` (mixes the two time axes).

### N8. Rule sets are stored statements

A rule set is a node `rs` (any IRI) with one live statement `R = (rs n3:source "<N3 text>")`; `store_rules` asserts it or supersedes the live one. Rule sets therefore have transaction time: `as_of(t)` reasons with the rules believed at `t`. `View::reason` takes either N3 text or a stored rule set. The namespace `n3:` is `urn:tiramemsu:n3:`; `tm:` is reserved for virtual predicates and cannot be written.

### N9. The rule-set statement is a support

Materialization derives every conclusion with support = its fact premises (eids), its derived premises (eids written earlier in the same commit) and `R`. Editing a rule set supersedes `R`, which retracts every conclusion it produced in the same transaction (design D4 of `add-derived-beliefs`); retracting it does the same. `(D n3:derivedBy rs)` and `(D n3:rule i)` are layers on `D` naming the rule set and the rule index; they are labels, not support.

- depends_on:: [[openspec:derived-beliefs#Support cascade]]

*Alternatives:* `derivedBy` as provenance only (old conclusions outlive edited rules).

### N10. Explicit materialization; own conclusions excluded

`Db::materialize` runs only when called. Its input excludes statements derived from the rule set's live `R` (they are recomputed); conclusions of other rule sets are ordinary facts, so rule sets layer.

*Alternatives:* automatic on commit (puts a reasoner in every write's critical section on the single writer).

### N11. Monotonic rule sets only for materialization

A static check over rule bodies classifies a rule set: `log:notIncludes`, `log:collectAllIn` and `log:forAllIn` make it non-monotonic; `time:localTime` and `tm:now` make it clock-dependent; `list:map`, whose proof does not list the facts it read, makes its premises untraced. `materialize` refuses all three with `N3NotMaterializable { rule, builtin }`; `reason` accepts every builtin. A fired inference fuse (`=> false`) is reported by `reason` as a contradiction and aborts `materialize` with `N3Contradiction` before anything is written.

*Alternatives:* reconcile by diffing runs (stored conclusions become "grounded as of the last run"; needs a new retraction kind); allow and document staleness.

### N12. Merge segments only with identical support

Within a segment a conclusion's support key is its premise eids plus, recursively, the keys of derived premises plus `R`. Adjacent segments merge into one interval only when content and support key are equal; otherwise they stay separate statements (episodes). eyeron returns one proof per conclusion per segment, so a stored conclusion may rest on one of several possible supports: it is sound, and a later `materialize` restores it if that support goes while another remains.

### N13. Reason on a snapshot, commit briefly

`materialize` reasons on a read view at `t₀` outside the writer, then opens one write transaction that writes conclusions in proof order. A conclusion with a support that is no longer live is skipped (with everything that depends on it) and reported as `stale_support`. The report carries `reasoned_at = t₀` and the commit's `TxReport`. `QueryBudget` bounds loading and the commit; eyeron's own limits bound reasoning; cancellation is observed between segments. `dry_run` previews the commit.

*Alternatives:* reasoning inside the writer (blocks every write for the run); optimistic retry (retry storms for no soundness gain).

### N14. Skolem nodes through a derived upsert

A blank node in a conclusion becomes `upsert_derived(n3:skolem, "<R eid>/<rule i>/<label>", support)`, where `label` is eyeron's deterministic witness name and `support` is that rule firing's support. Re-runs reuse the node; when the support goes, the key goes too.

- depends_on:: [[openspec:derived-beliefs#Derived upsert]]

### N15. Structured proofs always, N3 proofs on request

Every conclusion carries `{ rule_set, rule, premises }`; `proof_n3: true` adds eyeron's N3 proof text. Materialized proofs are the support links, never stored text.

### N16. Widening

When a live derivation with the same content and support exists and the new interval overlaps or touches it without being contained, `materialize` supersedes it to the hull of the two intervals. Intervals grow across runs and never shrink; `require_complete` refuses to commit an `Incomplete` run.

- depends_on:: [[openspec:derived-beliefs#Widening a derivation]]

### N17. Every surface, no query-language function

Rust, bridge, Node, Python, wasm, MCP and CLI expose `reason`, `store_rules` and `materialize`; SPARQL and Cypher read the materialized statements, `sys:derivedFrom` and `n3:derivedBy` like any data.

### N18. Tests prove the semantics

eyeron's example corpus through the store (conformance of the mapping); a property test that a sweep agrees with snapshots at random instants; a property test that every live conclusion's support is live after random retractions and that re-materializing converges; scenario tests for each decision; one benchmark scenario on both backends.

## Constraints

- **Never forget** constrains materialization: nothing is deleted; widening and rule edits are supersedes.
- **Single writer** ([[architecture#Connections and Concurrency]]) constrains N10 and N13: reasoning never runs on the commit path.
- **crates.io publishing** constrains N2: no git dependencies in published crates.
- **eyeron's license (MIT)** is compatible with the workspace's MIT OR Apache-2.0.

## Risks / Trade-offs

- [Sweeps multiply cost by the number of segments] → pruning, `max_segments`, and statements with unbounded validity form one segment; measured by the bench.
- [One proof per conclusion] → documented; re-materializing restores conclusions.
- [Upstream may not accept the PR] → the git pin keeps working; the facade feature stays unpublished.
- [Large rule-set edits retract many conclusions] → split rule sets by concern; `dependents(R)` previews it.

## Open Questions

- Lazy loading through a fact-source trait upstream, once the bench shows the load phase dominating.
- Reconciliation for non-monotonic rule sets (a `rederive` retraction kind).
