---
type: Decision
openspec: [derived-beliefs]
---
# Derived Beliefs

Decisions D45–D52 of the OpenSpec change `add-derived-beliefs` (2026-10-28): beliefs written with support links, so that a conclusion falls with its evidence.

They are D1–D5 and D8–D10 of the change's `design.md`; its D6 and D7 (read-time salience) moved to the change `add-read-time-salience`. The data model is in [[data-model#Derived Beliefs]], the walk in [[time-model#Cascade]].

## Support Links Extend The Referent Relation

D45 (design D1): `sys:derivedFrom` is engine-owned, its subject is always the derived statement, and the object of a live link counts as a referent of that subject, so the cascade of any support reaches it.

- motivated_by:: [[constraints#Grounded Memory]] {weight: primary, since: "2026-10-28"}

The cascade walk takes, from a reached statement `x`, every live statement whose subject or object is `x` (unchanged), and every live `D` with a live link `(D sys:derivedFrom x)`. In the paper this is the same ground-core construction over the extended referent relation, so the cascade stays the least grounded retraction ([[overview#Formal Model]]). The code is `Tx::derive` in `crates/tm-core/src/engine/derive.rs`.

Rejected: a belief written as a layer on its support (one support only, and the belief would no longer be about its node); a post-commit hook (not atomic, invisible to dry runs and `dependents`); a separate support table (a second structure outside the graph).

## Conjunctive Within A Derivation

D46 (design D2): one `derive` states that a belief follows from all its supports, so losing any one retracts it; two derivations of the same content are two statements, and the content stays live while one stands.

- motivated_by:: [[derived-beliefs#Support Links Extend The Referent Relation]] {weight: primary, since: "2026-10-28"}

Disjunction is visible in the data and needs no flag. Rejected: a `sys:derivedFromAny` predicate (two semantics in one predicate family) and retracting only when every support is gone (wrong for summaries, which rest on all their inputs).

## Idempotent Per Support Set

D47 (design D3): `derive` returns `Existing(D)` only for a live statement with the same content, an overlapping interval and exactly the same live support set; otherwise it creates a statement, also over an identical independent fact.

- motivated_by:: [[derived-beliefs#Conjunctive Within A Derivation]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[constraints#Grounded Memory]] {weight: secondary}

An independently asserted fact never acquires support links, so it never falls with someone else's evidence. Schema checks apply as for `create`: `sys:unique` rejects a duplicate subject, and `sys:one` retracts other objects, which may be independent facts. Rejected: attaching links to an existing identical statement, and failing with an error (agents would have to check first).

## Supersede Drops Derived Dependents

D48 (design D4): when supersede computes the cascade set of a root, a derived statement reached through a support link (never the root) is retracted with kind `supersede` and not replayed, nor are its structural dependents.

- motivated_by:: [[derived-beliefs#Support Links Extend The Referent Relation]] {weight: primary, since: "2026-10-28"}

This is the rule supersede already applies to foreign graph memberships ([[time-model#Operations#Supersede]]). A summary computed from the old content must be recomputed by the agent. Superseding the derived statement itself replays its links onto the new eid, because its support did not change. Rejected: replaying derived statements with links mapped through σ, which would silently keep a conclusion whose input changed.

## Bundles Carry Derivations

D49 (design D5): support links travel in fact bundles, their supports come with them through the downward closure, and import writes each derived statement through `derive` with its supports mapped to the target's eids.

- motivated_by:: [[derived-beliefs#Idempotent Per Support Set]] {weight: primary, since: "2026-10-28"}

`sys:derivedFrom` joins `sys:inGraph` as an engine-owned predicate that bundles keep. Import orders a derived statement after its supports, and a second import finds the same derivation and changes nothing ([[data-model#Fact Bundles]]).

## Both Engines One Oracle

D50 (design D8): derive, derived upsert, the support cascade, dependents, the supersede rule and bundle import exist in the SQL engine and in the trait engine, statement for statement, and the oracle runs derivations on both.

- motivated_by:: [[constraints#Same Logical Database]] {weight: primary, since: "2026-10-28"}
- motivated_by:: [[backends#One Engine In Two Phases]] {weight: secondary}

The SQL engine is `crates/tm-core/src/engine/derive.rs`, the trait engine (redb) `crates/tm-store/src/engine/derive.rs`. The random-history oracle generates derive and derived-upsert operations, and a fixed derived history runs on the in-memory store and on redb.

## Derived Upsert For Invented Nodes

D51 (design D9): `upsert_derived(p, o, support)` returns the node of a live key `(n p o)` derived from exactly `support`, or a new node whose key is written through `derive`; asserted keys and keys with other support are never reused.

- motivated_by:: [[derived-beliefs#Idempotent Per Support Set]] {weight: primary, since: "2026-10-28"}

A rule engine names the node it invents for an existential with a deterministic key, so re-running the rule reuses it. When the support goes, the key goes with the conclusions about the node, and the node, never deleted, is unused. Rejected: `upsert` with an asserted key (it outlives the conclusions and pins the node) and a fresh node per run (breaks idempotency).

## Widening Is A Supersede

D52 (design D10): a derived statement's valid interval widens through `supersede` with a patch that changes only valid time; the replay keeps its support links, so a conclusion's interval grows across runs.

- motivated_by:: [[derived-beliefs#Supersede Drops Derived Dependents]] {weight: primary, since: "2026-10-28"}

A conclusion that stops holding loses support and is retracted rather than narrowed.
