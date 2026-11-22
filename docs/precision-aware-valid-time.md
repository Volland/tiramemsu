# Precision-aware valid time

Status: proposal, 2026-11-22. Not implemented and not agreed. It comes from reading Positorium ([prior art](../lat.md/prior-art.md)).

## Problem

A statement's valid time is `[v_from, v_to)` in epoch milliseconds. "Alice joined in 2024" and "Alice joined on 2024-01-01 at midnight" both become `v_from = 2024-01-01T00:00:00Z`. After import the two claims cannot be told apart, so a query cannot ask "which dates are only known to the year" and a conflict check cannot tell "2024" overlapping "2024-05-17" (compatible) from two precise instants (a clash). Sanctions, ownership and other regulated data are full of partial dates.

## What Positorium does

A posit's time is a pair `(value, precision)`. A finite time is the half-open interval at its stated precision: `2024` is `[2024-01-01, 2025-01-01)`, `2024-05` is `[2024-05-01, 2024-06-01)`. Two stored times are equal only if value and precision both match. Source: its `docs/reference/MODEL.md`, section "Temporal meaning". Its time is a single "since when" per posit, not a closed `[from, to)` pair.

## Options for Tiramemsu

| Option | Change | Cost |
|---|---|---|
| A. Layer | A statement about the statement: `eid sys:validFromPrecision "year"` (and `sys:validToPrecision`). The interval columns still hold the expanded bounds. | No storage format change. Importers and `assert` helpers must write both. Queries need a join to read it. |
| B. Columns | Two small-int columns beside `v_from` and `v_to`. | Storage format bump, migration, every backend and the SQLite triggers. Cheapest to query. |
| C. Do nothing | Keep intervals only. | Precision stays lost. |

## Recommendation

Start with A. It needs no format change, it fits the layers model ([[data-model#Layers]] in `lat.md/`), and it shows whether anyone queries by precision before we pay for B. Concretely:

1. A parse helper that turns `"2024"`, `"2024-05"` and `"2024-05-17"` into the expanded interval plus a precision name.
2. A vocabulary term for the precision layer, written in the same transaction as the statement.
3. A recipe: dates known only to the year, and overlap of a year-precision claim with a day-precision one.
4. Revisit B only if the join becomes a cost in the benchmarks.

## Open questions

- Should a closed interval carry one precision or one per bound? Positorium needs only one because it has no `v_to`.
- Does `validAt` on a year-precision statement match any instant in the year? (Probably yes, since the interval is expanded.)
- How should the Allen interval relations of `add-interval-algebra` treat a precision layer: ignore it, or flag results that depend on it?
