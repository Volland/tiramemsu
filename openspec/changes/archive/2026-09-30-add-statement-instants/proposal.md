## Why

Every statement records the transactions that added and retracted it (`t_add`, `t_ret`), and every transaction records the wall-clock instant it committed (`tx(t, instant)`). The virtual predicates `tm:txAdded` and `tm:txRetracted` expose the transaction numbers, but the instants are unreachable from SPARQL and Cypher: a query can say "added in tx 205" but not "added on 2026-03-10".

That gap blocks the questions only a per-statement bitemporal store can answer, because they compare the two clocks of one statement:

- **Learned late:** facts that were recorded long after they became true (`addedAt` minus `validFrom` greater than N days). This measures ingestion lag and flags backfilled data.
- **Recorded after it stopped being true:** facts whose recording happened after the end of their valid interval (`addedAt > validTo`). These are purely historical records, never true while the store knew them.

Both need the transaction-time instant next to the valid-time bounds, as values of the same kind, in the same row.

## What Changes

- Two new statement-level virtual predicates:
  - `tm:addedAt`: the instant of the statement's `t_add`, as a `DATETIME` with offset `Z` (the same encoding as `tm:validFrom`).
  - `tm:retractedAt`: the instant of its `t_ret`, absent while the statement is live.
- They are computed with a scalar subquery on `tx` by its integer primary key, so they cost one rowid seek per row and no join. A constant object (`?r tm:addedAt "…"^^xsd:dateTime`) compares by instant and seeks the `tx_instant` index, then `t_add` or `t_ret`. A constant of another kind makes the pattern empty.
- They work under every view, like `tm:txAdded` and `tm:txRetracted`. Under `asOf`, `tm:retractedAt` shows a later retraction, as `tm:txRetracted` does.
- **SPARQL:** `?r tm:addedAt ?when`, `FILTER` comparisons with date-times by instant.
- **Cypher:** `r.addedAt` and `r.retractedAt` on a relationship or `:Statement` node return DateTimes. They are statement metadata like `txAdded`: a stored property of the same name stays reachable by CURIE, they are never listed by `keys()`, and `SET` or `REMOVE` of them fails with `Unsupported`.
- The JSON bridge needs no change: it passes SPARQL and Cypher through.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `virtual-predicates`: the table of recognised virtual predicates gains `tm:addedAt` and `tm:retractedAt`, and a new requirement states how their value is computed and compared.
- `cypher-temporal-clauses`: the statement time properties gain `addedAt` and `retractedAt`, which are read-only like `txAdded`.

## Impact

- **`tm-ir`:** two IRIs in `vocab` and in the `VIRTUAL` table.
- **`tm-exec`:** two `VirtualPred` variants whose object expression and constant compare use a subquery on `tx`. No planner, view or storage change.
- **`tm-cypher`:** the temporal names table, property access, pattern property maps, and the read-only rule in `SET`/`REMOVE`.
- **Not affected:** the storage format, the `tx` table (it already has `instant` and the unique `tx_instant` index), `tm-core`, the SPARQL lowering (virtual predicates are recognised by IRI), the path engine and the bindings.
- **Docs:** `lat.md/query.md` (Virtual Predicates, Cypher Dual View, Temporal Syntax) and `lat.md/tests.md`.
