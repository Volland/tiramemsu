## Context

See `proposal.md` (Why) for the motivation. This section covers the state and constraints that shape the design.

- **Virtual predicates** (`lat.md/query#Views and Scans#Virtual Predicates`, D6) are computed from the statement's own row by `tm-exec`'s `VirtualPred`: an object expression over the row alias, an optional `IS NOT NULL` guard, and a column compare for a constant object. When the subject is the eid of another pattern under the same view, the row alias is reused and no scan is added.
- **Transaction instants** live only in `tx(t INTEGER PRIMARY KEY, instant INTEGER NOT NULL)` with the unique index `tx_instant`. Instants are strictly increasing epoch milliseconds (`lat.md/time-model#Transaction Time`), so `t` and `instant` order the same way.
- **Valid-time bounds** are already exposed as `DATETIME` ids with offset `Z`: `(ms << 15) | DATETIME_Z_LOW`. Value comparison of date-times uses the instant (`id >> 15`), so a `tm:validFrom` value and any other date-time compare correctly without a dictionary lookup.
- **Cypher** treats `txAdded`, `txRetracted`, `validFrom` and `validTo` as statement metadata (`crates/tm-cypher/src/exec/access.rs`, `TEMPORAL`), with the rules of `cypher-temporal-clauses`.

## Goals / Non-Goals

**Goals:**
- The transaction-time instant of a statement, as a date-time comparable with `tm:validFrom` and `tm:validTo` in one row, in both dialects.
- No join and no new index: a rowid seek per row for a variable object, an index seek for a constant.
- The same view, visibility and absent-value rules as the existing virtual predicates.

**Non-Goals:**
- Date-time arithmetic in SPARQL (`?a - ?b`, `xsd:dayTimeDuration`). SPARQL 1.1 defines no date-time subtraction, and the executor has no duration values. The "learned late by more than N days" recipe is therefore Cypher-only (`epochMillis`), and SPARQL expresses "learned late" as `?added > ?from`.
- A transaction-level virtual predicate (`?t tm:instant ?when` on a `TX` id). Transactions are subjects of ordinary metadata triples; exposing their instant is a separate question.
- Writing instants. Transaction time is assigned by the engine.

## Decisions

### Decision 1: Two statement-level predicates, not a transaction-level one

`tm:addedAt` and `tm:retractedAt` take a statement as subject, like `tm:txAdded`. The alternative, `?r tm:txAdded ?t . ?t tm:instant ?when`, is one more pattern per use, needs a new kind of virtual subject (`TX` ids), and in Cypher would need a transaction value with properties. The recipes compare instants with `validFrom` on the same statement, so the statement is the natural subject.

### Decision 2: A scalar subquery on `tx`

The object expression is `(((SELECT tx.instant FROM tx WHERE tx.t = a.t_add) << 15) | DATETIME_Z_LOW)`. `tx.t` is the integer primary key, so the subquery is one rowid seek, and it keeps the virtual pattern a single `triple` alias, so alias reuse (a bound eid costs no extra scan) is unchanged. A join with `tx` in the FROM list would have to be threaded through `Rel` items and optional-join handling for no gain.

`tm:addedAt` is never absent: every `t_add` has a `tx` row. `tm:retractedAt` has the `t_ret IS NOT NULL` guard of `tm:txRetracted`.

### Decision 3: Constant objects seek by instant

A constant date-time compares by instant, whatever its offset. The compare is `a.t_add = (SELECT tx.t FROM tx WHERE tx.instant = ?)`: the subquery seeks the unique `tx_instant` index once, and the outer compare can use `log_add` (or `t_ret` for `tm:retractedAt`). Comparing `(SELECT instant … WHERE t = a.t_add) = ?` would be a correlated subquery per statement. Because instants are unique, the two forms are equal. An instant that no transaction has matches nothing. A constant of another kind (an integer, a transaction) makes the pattern empty at plan time, as for the other virtual predicates.

### Decision 4: Views and `asOf`

Visibility follows the pattern's view, as for every virtual predicate. The value is the row's own column: under `asOf(t)`, a statement retracted after `t` is visible and `tm:txRetracted` returns the later transaction. `tm:retractedAt` does the same, returning the instant of that later retraction. Hiding it would make the two predicates disagree, and the row's retraction is a fact about the statement, not about the view.

### Decision 5: Cypher names

`addedAt` and `retractedAt` join the statement time properties. They follow the `txAdded` rules exactly: metadata on statements even when a stored property of the same name exists (reachable as `` `v:addedAt` ``), ordinary keys on ordinary nodes, never in `keys()` or `properties()`, and `SET` or `REMOVE` fail with `Unsupported` because transaction time is read-only. A `CREATE` property map with these names writes an ordinary property, as `txAdded` does today. The names map to IRIs through one table in `access.rs`, which replaces the repeated `match` on the four names.

## Risks / Trade-offs

- **A subquery per row.** A variable `tm:addedAt` over many statements costs one rowid seek each. That is the cost of the data: the instant is in another table. The page holding a run of `tx` rows is hot, so the seeks are cheap. A constant object avoids it (Decision 3).
- **Non-UTC offsets are lost.** Instants are stored as epoch milliseconds, so the value always has offset `Z`, as `tm:validFrom` does.
- **SPARQL cannot threshold the lag.** See Non-Goals. The Cypher recipe covers it, and a later change can add `xsd:dayTimeDuration` arithmetic.

## Migration Plan

Additive. No format change and no data migration. A stored triple with predicate `tm:addedAt` cannot exist (the `tm:` namespace is reserved for user writes), so no query changes meaning. In Cypher, `r.addedAt` on a statement that had a stored `v:addedAt` property now returns the instant; the property stays reachable as `` r.`v:addedAt` ``, the same rule that applies to `txAdded`.

## Open Questions

- Should SPARQL gain `xsd:dayTimeDuration` arithmetic so "learned late by N days" is expressible there too? Left to a follow-up change.
