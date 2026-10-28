# Derived beliefs with support

Target release: **0.6 — Agent recall**. Source: `docs/feature-roadmap.md#0.6` ("Consolidation helpers"). Read-time salience, first proposed here, was split into `add-read-time-salience` on 2026-10-28; this change is the prerequisite of `add-n3-reasoning`.

## Why

Agents consolidate memory: they summarise twenty observations into one belief, infer a preference from repeated behavior, or store a computed answer. Today such a belief is an ordinary assertion. When its evidence is later retracted or corrected, the summary stays live and nothing marks it, so memory quietly keeps conclusions whose grounds are gone. The roadmap's sketch assumed that the existing cascade would handle this if summaries carried `sys:derivedFrom` links. It does not: the cascade (`crates/tm-core/src/engine/cascade.rs#cascade_set`) follows statements whose subject or object is a retracted eid, so retracting the support `e1` retracts the link `(D sys:derivedFrom e1)` (its object is `e1`) but not the summary `D`, whose subject is a node. The engine needs one rule more.

A rule engine (`add-n3-reasoning`) has the same need at scale: each conclusion rests on the facts and the rule that produced it, and must fall when any of them goes.

## What Changes

- **`Tx::derive(s, p, o, valid, support)`** asserts a belief together with one engine-owned link `(D sys:derivedFrom e)` per supporting statement. Support must be non-empty, live, and not `D` itself; supports may themselves be derived. Re-deriving the same content from the same support set is idempotent (`Existing`); deriving the same content from a different support set, or over an independently asserted identical statement, creates a parallel statement, so an independent assertion never becomes dependent and several derivations of one belief express alternative grounds.
- **Support cascade.** A support link makes the support a referent of the derived statement: when a live link `(D sys:derivedFrom e)` is retracted because `e` is retracted, `D` is retracted in the same transaction with the same `ret_kind` rule as any cascade member, recursively. Within one derivation support is conjunctive (losing any support retracts the belief); across parallel derivations it is disjunctive. `View::dependents`, dry runs and `max_cascade` follow the same rule, so "what breaks if I retract this" includes the conclusions drawn from it.
- **Supersede.** Correcting a support retracts the derived statements that rest on it and does not replay them (they were computed from the old content), the way supersede already drops foreign graph memberships; correcting the derived statement itself replays its support links onto the new eid.
- **Saved answers** that cited a derived statement go `Stale` through the existing `supportRetracted` cause when the support cascade retracts it; no change to saved-answer code is needed beyond tests.
- **Bundles** carry derivations (the links are layers, and their objects are in the downward closure) and import re-derives them through `derive`.
- **Surfaces:** JSON bridge transaction op `derive`, Node `Tx.derive`, Python `TxBuilder.derive`, MCP tool `derive` (a write tool, hidden in read-only mode); SPARQL and Cypher read `sys:derivedFrom` like any predicate (it is engine-owned, so direct writes are `ReservedNamespace`).
- **`Tx::upsert_derived(p, o, support)`** finds or creates a node by a key that is itself derived from `support`, so a rule engine's invented (Skolem) nodes are reused across runs and their key falls with their support.
- **Widening.** Superseding a derived statement with only a new valid interval keeps its support; a rule engine widens a conclusion's interval this way.
- **Both engines.** Everything above is implemented in the SQL engine (`tm-core`) and the trait engine (`tm-store`, redb), with the oracle tests on both.

## Capabilities

### New Capabilities

- `derived-beliefs`: the `derive` and `upsert_derived` verbs, support links, idempotency rules, supersede, widening and bundle behavior.

### Modified Capabilities

- `retraction-cascade`: "Cascade over subject and object positions" also follows support links to the derived statement.
- `statement-dependents`: "Dependents of a statement" and "Dependents equal the cascade" include derived statements reached through support links.

## Impact

- **`crates/tm-core` (SQL engine) and `crates/tm-store` (trait engine):** `vocab.rs` (`SYS_DERIVED_FROM`), `engine/reserved.rs` (engine-owned like `sys:inGraph`), new `engine/derive.rs` (`Tx::derive`, `Tx::upsert_derived`), `engine/cascade.rs` (the cascade adds `D` when a live `sys:derivedFrom` link to a member is reached), `engine/supersede.rs` (drop derived statements reached through support; replay links when the derived statement is the root), `read.rs#dependents` (same rule), bundle export and import.
- **Facade, bindings, MCP:** `crates/tiramemsu` `Tx::derive`/`upsert_derived`, `bindings/json/src/tx.rs` (`derive`), Node, Python, `crates/tiramemsu-mcp/src/tools.rs` (`derive`).
- **Formal model:** `paper/` and `lat.md/overview.md#Formal Model` note that support links extend the referent relation, so the cascade remains the least grounded retraction under it.
- **Not affected:** storage format (support links are ordinary statements).

## Non-goals

- No summarisation, inference or rule engine in the store; the caller decides what to derive and from what.
- No disjunctive support inside one derivation (use parallel derivations).
