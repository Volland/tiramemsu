## Context

See proposal.md. `crates/tiramemsu/src/n3_materialize.rs` implements N3 materialization: it reasons on a snapshot view, then in one transaction derives conclusions in proof order through `Tx::derive`, maps derived premises to the eids just written, skips stale support with its dependents, detects existing and widenable derivations by content and support, and turns blank nodes into skolem nodes through `upsert_derived`. `add-derived-beliefs` gives `derive`, the support cascade, `dependents` and widening by supersede. `add-datalog-engine` gives evaluation with one derivation per fragment (provenance on request).

## Goals / Non-Goals

**Goals:** one materialization core for both languages; non-monotonic strata stored and kept consistent at every run.

**Non-Goals:** keeping conclusions consistent between runs for non-monotonic strata (reported, not enforced); multiple stored derivations per conclusion.

## Constraints

- [[constraints#Grounded Memory]] constrains materialization {strength: must}: every stored conclusion rests on live support at commit.
- [[constraints#Never Forget]] constrains reconciliation {strength: must}: it retracts, never deletes.
- [[constraints#Single Writer]] constrains the commit {strength: must}: evaluation stays outside the writer.

## Decisions

### DL13a. One materialization core

`src/materialize.rs` takes a list of conclusions `{content, valid, premises: [Eid | Derived(i)], rule, invented}` in proof order and a label policy, and does the commit work shared by both languages: stale-support closure, existing and widening detection, `derive`, skolems, layers, reconciliation retractions, labels. `n3_materialize.rs` maps eyeron's proofs to that list; `datalog_materialize.rs` maps the engine's derivations.

- motivated_by:: [[reasoning#Snapshot Then Commit]] {weight: primary}

*Alternatives:* a second copy for Datalog (two implementations of grounding to keep equal).

### DL13b. Reconciliation set

For each non-monotonic stratum, the run computes the program's live conclusions of that stratum's relations (statements with a live `sys:derivedFrom R` link and a `dl:rule` of that stratum) and retracts those not matched by the run's output on content, support and interval. Retraction is `Tx::retract`, so its own dependents cascade.

- motivated_by:: [[reasoning#Monotonic Materialization]] {weight: primary, note: "closes the open follow-up for Datalog"}
- motivated_by:: [[constraints#Never Forget]] {weight: secondary}

### DL14. Labels on the transaction

`(tx dl:reconciled program)` with `(tx dl:reconciledCount n)` and `(tx dl:materialized program)` are statements about the reified transaction. Retractions keep `ret_kind` 0. No format change.

### DL22 is not part of this change

Incremental evaluation (a delta of added statements seeding semi-naive from the program's live conclusions, with rederivation of what the cascade retracted) needs engine support that is not built; each run evaluates in full on a snapshot and writes only the difference through the commit core, so a re-run asserts nothing. The transaction label `(tx dl:materialized program)` is written already so that a later incremental run has its base.

### Fragments and support

The engine's interval sets are split into maximal fragments over which one derivation holds (the provenance side array records one derivation per fragment); each fragment is one `derive`. Merging of touching fragments happens only with identical support ([[reasoning#Merge By Support]]).

## Risks / Trade-offs

- [Reconciled strata go stale between runs] → `needs_rerun_after_writes` in the report; recipes show running materialize after writes that touch the strata's predicates.
- [Rederivation of many cascade-retracted conclusions] → demand-rewritten goals share one evaluation per relation; falls back to a full run past a threshold of 10 % of the program's conclusions.
- [Refactoring N3 materialization] → all N3 materialization scenario tests and property tests run unchanged before and after.

## Migration Plan

Additive. N3 behavior unchanged. No format change.
