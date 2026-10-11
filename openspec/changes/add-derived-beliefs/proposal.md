# Derived beliefs with support, and read-time salience

Target release: **0.6 — Agent recall**. Source: `docs/feature-roadmap.md#0.6` ("Consolidation helpers", "Salience without forgetting").

## Why

Agents consolidate memory: they summarise twenty observations into one belief, infer a preference from repeated behavior, or store a computed answer. Today such a belief is an ordinary assertion. When its evidence is later retracted or corrected, the summary stays live and nothing marks it, so memory quietly keeps conclusions whose grounds are gone. The roadmap's sketch assumed that the existing cascade would handle this if summaries carried `sys:derivedFrom` links. It does not: the cascade (`crates/tm-core/src/engine/cascade.rs#cascade_set`) follows statements whose subject or object is a retracted eid, so retracting the support `e1` retracts the link `(D sys:derivedFrom e1)` (its object is `e1`) but not the summary `D`, whose subject is a node. The engine needs one rule more.

Agents also need to rank what to bring back. The store holds access patterns only as volatile values (`volatile(s, key, value, updated_at)`, `crates/tm-core/src/engine/volatile.rs`) and recency as `tm:addedAt`, but has no way to combine them into a score, and the never-forget rule means forgetting cannot be the answer: low-salience memory should rank lower, not disappear.

## What Changes

- **`Tx::derive(s, p, o, valid, support)`** asserts a belief together with one engine-owned link `(D sys:derivedFrom e)` per supporting statement. Support must be non-empty, live, and not `D` itself; supports may themselves be derived. Re-deriving the same content from the same support set is idempotent (`Existing`); deriving the same content from a different support set, or over an independently asserted identical statement, creates a parallel statement, so an independent assertion never becomes dependent and several derivations of one belief express alternative grounds.
- **Support cascade.** A support link makes the support a referent of the derived statement: when a live link `(D sys:derivedFrom e)` is retracted because `e` is retracted, `D` is retracted in the same transaction with the same `ret_kind` rule as any cascade member, recursively. Within one derivation support is conjunctive (losing any support retracts the belief); across parallel derivations it is disjunctive. `View::dependents`, dry runs and `max_cascade` follow the same rule, so "what breaks if I retract this" includes the conclusions drawn from it.
- **Supersede.** Correcting a support retracts the derived statements that rest on it and does not replay them (they were computed from the old content), the way supersede already drops foreign graph memberships; correcting the derived statement itself replays its support links onto the new eid.
- **Saved answers** that cited a derived statement go `Stale` through the existing `supportRetracted` cause when the support cascade retracts it; no change to saved-answer code is needed beyond tests.
- **Bundles** carry derivations (the links are layers, and their objects are in the downward closure) and import re-derives them through `derive`.
- **Surfaces:** JSON bridge transaction op `derive`, Node `Tx.derive`, Python `TxBuilder.derive`, MCP tool `derive` (a write tool, hidden in read-only mode); SPARQL and Cypher read `sys:derivedFrom` like any predicate (it is engine-owned, so direct writes are `ReservedNamespace`).
- **Read-time salience.** A deterministic score `salience = (1 + ln(1 + count)) · 2^(−age / half_life)` per node or statement, where `count` is an integer volatile counter under a chosen key, and `age` is `now` minus the latest of the counter's `updated_at` and the newest `tm:addedAt` of the target's live statements. It is exposed as `View::salience(targets, &SaliencePolicy)`, SPARQL `tm:salience(?x, <counterKey>, "P7D"^^xsd:dayTimeDuration)`, Cypher `tiramemsu.salience(x, {key, halfLife})`, and the bridge op `salience`, usable in `ORDER BY` and `FILTER`. `Tx::increment_volatile(s, key, by)` records an access atomically. Salience never writes and never retracts.

## Capabilities

### New Capabilities

- `derived-beliefs`: the `derive` verb, support links, idempotency rules, supersede and bundle behavior.
- `read-time-salience`: the salience score, its policy, query functions and the volatile counter increment.

### Modified Capabilities

- `retraction-cascade`: "Cascade over subject and object positions" also follows support links to the derived statement.
- `statement-dependents`: "Dependents of a statement" and "Dependents equal the cascade" include derived statements reached through support links.

## Impact

- **`crates/tm-core`:** `src/vocab.rs` (`SYS_DERIVED_FROM`), `src/engine/reserved.rs` (engine-owned like `sys:inGraph`), new `src/engine/derive.rs` (`Tx::derive`), `src/engine/cascade.rs` (`cascade_set` adds `D` when a `sys:derivedFrom` link to a member is reached), `src/engine/supersede.rs` (drop derived statements reached through support, propagate exclusion to their dependents, as at lines ~176–210), `src/read.rs#dependents` (same rule), `src/bundle.rs` and `src/engine/bundle.rs` (keep links, import through `derive`), `src/engine/volatile.rs` (`increment_volatile`), new `src/salience.rs` (score and policy, pure function shared by all surfaces).
- **`crates/tm-exec`:** `src/udf.rs` (`tm_salience` scalar UDF over count, age and half-life), `src/sqlgen/expr.rs` (join of `volatile` and the newest `t_add` instant), `crates/tm-ir/src/expr.rs` (`Func::Salience`).
- **`crates/tm-sparql`:** `src/lower/expr.rs` (`tm:salience`); **`crates/tm-cypher`:** `src/exec/funcs.rs`, `src/funcs.rs`.
- **Facade, bindings, MCP:** `crates/tiramemsu/src/view.rs` (`View::salience`), `bindings/json/src/{tx,read}.rs` (`derive`, `incrementVolatile`, `salience`), Node, Python, `crates/tiramemsu-mcp/src/tools.rs` (`derive`).
- **Formal model:** `paper/` and `lat.md/overview.md#Formal Model` note that support links extend the referent relation, so the cascade remains the least grounded retraction under it.
- **Not affected:** storage format (support links are ordinary statements; salience is computed). `add-hybrid-recall` (another change) may use the salience score as an evidence component; this change only provides it.

## Non-goals

- No summarisation, inference or rule engine in the store; the caller decides what to derive and from what.
- No automatic decay that retracts or hides anything.
- No disjunctive support inside one derivation (use parallel derivations).
