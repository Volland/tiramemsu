## Context

The cascade retracts, recursively, every live statement whose subject or object is a retracted statement (`lat.md/time-model.md#Cascade`; `crates/tm-core/src/engine/cascade.rs#cascade_set`, breadth-first in chunks of 500). `View::dependents` is the same walk as a read (`crates/tm-core/src/read.rs#dependents`), equal to a dry-run retraction and to the ends of the path `(^sys:subject|^sys:object)*`. The paper (`paper/layered-bitemporal-graphs.tex`) proves the cascade retracts exactly X ∖ γ(X ∖ {e}), the least retraction that keeps memory grounded, where a statement's referents are its statement-valued subject and object.

A derived belief `D = (alice v:prefers v:tea)` is about a node, so no support `e` is a referent of `D`. A link `L = (D sys:derivedFrom e)` has `D` as subject and `e` as object: retracting `e` retracts `L`, and retracting `D` retracts `L`, but retracting `e` never reaches `D`.

Supersede already drops part of a cascade set instead of replaying it: foreign graph memberships and their structural dependents (`crates/tm-core/src/engine/supersede.rs`). `sys:inGraph` is engine-owned (`crates/tm-core/src/engine/reserved.rs`) and bundles import it through `add_to_graph`.

Volatile values are upserted per `(s, key)` with `updated_at` = the transaction instant and are visible only under the now view.

## Goals / Non-Goals

**Goals:**
- Retracting or correcting evidence retracts what was concluded from it, in the same transaction, visible in previews.
- Independent assertions never become dependent by accident.
- Conclusions of a rule engine (`add-n3-reasoning`) can be written as derived beliefs: invented nodes stay stable across runs and a conclusion's interval can widen.

**Non-Goals:**
- Inference, truth maintenance beyond support links, or scheduled decay.

## Decisions

### D1. Support links extend the referent relation

`sys:derivedFrom` is engine-owned and its subject is always the derived statement. For grounding, the object of a live `(D sys:derivedFrom e)` counts as a referent of `D`. The cascade walk becomes: from a reached statement `x`, take every live statement whose subject or object is `x` (unchanged), and every live `D` such that a live `(D sys:derivedFrom x)` exists. In the formal model this is the same ground-core construction over the extended referent relation, so the cascade stays the least grounded retraction; the paper gains a remark, not a new proof.

*Alternatives:* model the belief as a layer whose subject is the support (only works for one support, and the belief is no longer about its node); retract derived statements in a post-commit hook (not atomic, invisible to dry runs and `dependents`); a separate `support` table (a second structure outside the graph, against the store's single-model design).

### D2. Conjunctive within a derivation, disjunctive across derivations

One `derive` call states "this follows from all of these". Losing any support retracts the belief. Two calls with different support sets for the same content create two statements, and each falls with its own support, so the content stays live while any derivation stands. This needs no new flag and is visible in the data.

*Alternatives:* a per-link `sys:derivedFromAny` (two semantics on one predicate family); retract only when all supports go (wrong for summaries, which rest on all their inputs).

### D3. Idempotency only for the same support set

`derive` looks for a live statement with the same `(s, p, o)`, overlapping valid time and exactly the same live support set; if found it returns `Existing`. Otherwise it creates a new statement (as `create` does), even when an identical non-derived statement is live. Schema checks apply as for `create` (`sys:unique` rejects a duplicate subject; `sys:one` retracts other objects, which may be independent facts — documented).

*Alternatives:* attach links to an existing identical statement (makes an independently asserted fact dependent; one retraction would remove a fact the agent observed directly); fail with an error (forces agents to check first).

### D4. Supersede drops derived dependents of a corrected support

When supersede computes the cascade set of a root, any derived statement reached through a support link (never the root itself) is selected for dropping, and exclusion propagates through its structural dependents, exactly as for foreign memberships. Those statements are retracted with `ret_kind = supersede` and not replayed: a summary computed from the old content must be recomputed by the agent. Superseding a derived statement itself replays its support links onto the new eid, since its support did not change.

*Alternatives:* replay derived statements with links mapped through σ (keeps a conclusion whose input changed, silently).

### D5. Bundles carry derivations

Support links are layers of `D`; their objects are statements, so the bundle's downward closure already includes the supports. `sys:derivedFrom` joins `sys:inGraph` as an engine-owned predicate that bundles keep, and import writes each derived statement through `derive` with its supports mapped to target eids.

### D8. Both engines, one oracle

`derive`, the support cascade, `dependents` and the supersede rule are implemented in the SQL engine (`tm-core`) and in the trait engine (`tm-store`, used by redb), statement for statement, and the existing oracle tests run every new scenario on both, as for every other write ([[backends#One Engine In Two Phases]]).

### D9. Derived upsert for invented nodes

`Tx::upsert_derived(p, o, support)` finds the node `n` with a live derived key `(n p o)` or creates a new node and writes the key through `derive` with `support`. A rule engine that invents a node for an existential (a Skolem node) calls it with a deterministic key, so re-running the rule reuses the node and `derive` stays idempotent; when the key's support goes, the key is retracted with the conclusions about the node, and the node, which is never deleted, is simply unused. A plain `upsert` key is independent, so the two never share a key statement: `upsert_derived` ignores keys that are not derived from exactly `support`.

*Alternatives:* `upsert` with an asserted key (an independent key outlives the conclusions and pins the node forever); a fresh node per run (breaks idempotency).

### D10. Widening a derivation is a supersede

A derived statement's valid interval may be widened by `Tx::supersede` with a `Patch` that changes only the valid time; D4's replay keeps the support links. A rule engine that finds the same content with the same support over a longer interval than the stored one uses this, so a conclusion's interval grows across runs and never shrinks: a conclusion that stops holding loses support and is retracted, not narrowed.

## Risks / Trade-offs

- [Support cascades can be large] → `max_cascade` applies to the extended walk; `dependents` and dry runs show the full set first.
- [`sys:one` on a derived predicate retracts independent facts] → Documented; agents should derive into their own predicates or use `confirm`.
- [Parallel derivations duplicate rows in Cypher's bag view] → Expected: each derivation is a distinct belief; SPARQL's set view shows one triple.

## Migration Plan

No format change. `sys:derivedFrom` was previously a `ReservedNamespace` error, so no existing file holds it. The cascade change only affects stores that use `derive`. The paper and `lat.md` formal-model notes are updated in the same change.

## Open Questions

- Should `Tx::add_support(D, e)` exist to extend a derivation later, or must a new derivation be created?
- Should derived statements carry the deriving method (`(D v:method …)`) by convention or by a required argument? `add-n3-reasoning` answers it for rule engines by convention: the rule set's statement is one of the supports and `tm:derivedBy` labels the rule.
