# N3 reasoning with eyeron, aware of valid and transaction time

Source: design interview of 2026-10-28 (decisions N1–N18 in `design.md`). Depends on `add-derived-beliefs` (hard prerequisite for materialization) and, for publishing, on `publish-eyeron-dependency`.

## Why

Memory that only stores what it was told cannot say what follows from it. Notation3 rules are the RDF world's way to write "if these facts hold, this one holds too", and [eyeron](https://github.com/eyereasoner/eyeron) is a native Rust N3 reasoner that returns, with every conclusion, the rule and the premises it used. Tiramemsu can add two things no N3 reasoner has on its own: conclusions that respect **valid time** (two facts that were never true at the same time never combine) and **transaction time** (reason over memory as it was believed at any past instant), and conclusions that are **grounded**: written as derived beliefs whose support is the premises and the rule set, so they fall automatically when a premise or a rule is retracted or corrected.

## What Changes

- **New optional crate `tm-n3`** on `tm-core` and `eyeron`, behind a facade feature `n3` that is off by default and independent of `exec` (the core tier can reason).
- **Read-only reasoning:** `View::reason(rules, &ReasonOptions)` loads a scope of the view (whole view, named graphs or seed nodes), pruned to the predicates the rules use, runs eyeron and returns conclusions with structured proofs (rule set, rule index, premise eids) and a completion status; N3 proof text on request.
- **Pointwise valid time:** a conclusion holds at instant `t` iff it follows from the statements valid at `t`. `valid_at(t)` views reason over one snapshot; otherwise the run sweeps the segments between the valid-time bounds of the loaded statements and merges adjacent segments with the same conclusion and support. A segment limit reports `Incomplete`.
- **Transaction time:** reasoning runs on any view (`now`, `as_of`), and `time:localTime` returns the view's instant, fixed once per run, so as-of reasoning is reproducible.
- **Statements in N3:** a statement is `<urn:tiramemsu:stmt:N>` with `tm:content { s p o }`, `tm:validFrom` and `tm:validTo` available to rules, generated when a rule body uses them; nodes are `<urn:tiramemsu:node:N>`.
- **Stored rule sets:** `Tx::store_rules(rule_set, n3)` keeps a rule set as one statement `(rule_set n3:source "…")`, changed by supersede, so `as_of` also selects the rules believed then. A static check marks rule sets that use non-monotonic or clock-reading builtins.
- **Materialization:** `Db::materialize(rule_set, &MaterializeOptions)` reasons on a snapshot and writes conclusions in one short transaction through `derive`, with the premises **and the rule-set statement** as support and an `n3:derivedBy` label. Editing or retracting a rule set, or retracting or correcting a premise, retracts the conclusions through the support cascade. Re-runs are idempotent, widen intervals, reuse invented (Skolem) nodes through `upsert_derived`, skip conclusions whose support was retracted meanwhile (`stale_support`), refuse non-monotonic rule sets and abort on a fired inference fuse.
- **Every surface:** Rust facade, JSON bridge (`reason`, `storeRules`, `materialize`), Node, Python and wasm through the bridge, MCP tools (`reason` read; `store_rules`, `materialize` write, hidden in read-only mode) and the CLI (`tm reason`, `tm rules`, `tm materialize`). No SPARQL or Cypher function: materialized conclusions are ordinary statements every dialect reads.

## Capabilities

### New Capabilities

- `n3-reasoning`: read-only reasoning over a view, scope and pruning, the statement mapping, pointwise valid time, the pinned clock, proofs and completion status.
- `n3-rule-sets`: storing, superseding and reading rule sets, the monotonicity check, and rule sets as of a past instant.
- `n3-materialization`: snapshot-then-commit materialization through `derive`, rule set as support, segment merging, Skolem nodes, widening, refusals and the surfaces.

### Modified Capabilities

- `optional-query-frontends`: the facade gains the `n3` feature, off by default.

## Impact

- **New crate `crates/tm-n3`:** mapping, pruning, segment sweep, eyeron driver, proofs, monotonicity check. Workspace dependency `eyeron` pinned to a git commit until `publish-eyeron-dependency` lands; `tm-n3` is `publish = false` until then.
- **`crates/tiramemsu`:** feature `n3`; `View::reason`, `Tx::store_rules`, `Db::materialize` (`src/n3.rs`).
- **Bindings, MCP, CLI:** `bindings/json` (feature `n3`, ops), `bindings/node`, `bindings/python`, `bindings/wasm`, `crates/tiramemsu-mcp/src/tools.rs`, `crates/tiramemsu-cli`.
- **lat.md:** new `lat.md/reasoning.md` (typed `Decision`), `lat.md/architecture.md#Crates`, `lat.md/tests.md` (N3 sections), `lat.md/time-model.md#Valid Time` (reference). Binding decisions: [[time-model#Valid Time]], [[data-model#Named Graphs]], the never-forget rule, [[query#Query Budgets]], [[backends#One Engine In Two Phases]].
- **Site:** a reasoning page and a mini book on N3 reasoning with tiramemsu.
- **Bench:** a `tm-bench` scenario for load, pruning and sweep cost.

## Non-goals

- No reasoning on the commit path: nothing runs automatically after a write.
- No non-monotonic materialization (negation as failure, aggregates); read-only `reason` supports them.
- No SPARQL or Cypher reasoning function, no OWL profile, no backward chaining over the store (eyeron's backward rules run over the loaded scope only).
- No network or file access from rules.
