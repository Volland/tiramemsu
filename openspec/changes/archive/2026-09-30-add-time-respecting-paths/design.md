## Context

- **Valid time** is `[v_from, v_to)` per statement, epoch ms, NULL unbounded (`lat.md/time-model#Valid Time`). It is immutable content of the statement, so a hop's interval is known from the row the fetcher already reads.
- **The path engine** is a BFS over `(node, DFA state)`, one batched fetch per layer (`lat.md/query#Physical Planning#Path Engine`). `REACH` keeps a visited set and emits each end at its first layer; `TRAIL` keeps an arena of partial trails; the shortest modes keep a layered DAG of first-discovery predecessors. Rows come in non-decreasing hops with a deterministic order inside a layer.
- **The covering indexes** `live_*` and `hist_*` end with `v_from, v_to`, so reading the interval in the fetch changes no plan.
- **Temporal-graph literature** calls this a time-respecting (or causal) path, or a journey, and the usual question is the earliest arrival: the smallest time at which a target can be reached. Contact-sequence models (each contact at an instant) are the special case `[t, t+1)`.

## Goals / Non-Goals

**Goals:**
- Correct earliest-arrival semantics in every mode, checked against a brute-force enumeration of time-respecting walks.
- No change for a request that is not time-respecting: same rows, same order, same SQL plans, `arrival = None`.
- One option shared by the engine, `tm_path`, `View::path_with` and the bridge.

**Non-Goals:**
- SPARQL or Cypher syntax (Open Questions).
- Waiting-time bounds (a maximum dwell between hops), latest-departure and fastest (shortest-duration) journeys, and a duration per hop. Each is a different optimisation criterion.
- Using transaction time as the clock. The journey is in valid time; the view still selects the transaction-time state as today.

## Decisions

### Decision 1: Earliest-arrival hop rule

A search carries τ ∈ {−∞} ∪ ℤ (internally `i64::MIN` is −∞), starting at `after` or −∞. For a stored hop over `[v_from, v_to)`:

- allowed iff `v_to IS NULL OR v_to > τ` (the fact still holds at or after τ);
- then `τ' = max(τ, v_from)`, a NULL `v_from` leaving τ unchanged.

The hop is taken at the earliest instant ≥ τ at which the fact holds; waiting is free and unbounded. Equal instants chain (non-decreasing, not strictly increasing), so two facts valid from the same instant form a journey. Worked example: `A→B [1,5)` then `B→C [3,9)`: τ = −∞ → 1 → 3, reachable, arrival 3. With `B→C [0,2)` instead: at B τ = 1 and `2 > 1`, so the hop is allowed at instant 1, arrival 1. With `B→C [0,1)`: `1 > 1` fails, C is not reachable. With `after = 6` and `A→B [1,5)`: `5 > 6` fails.

Virtual hops read the parts of a statement, which exist for the statement's whole life, so they are always allowed and leave τ unchanged. The view's `validAt(d)` still keeps only statements valid at `d`; combined with time respect, the journey then uses only facts valid at `d`, which is allowed but rarely wanted, and documented.

Alternatives considered:
- *Strictly increasing times* (`v_from > τ`). Rejects simultaneous events and makes `[t, t+1)` contacts at the same `t` unchainable; most temporal-network definitions use non-decreasing.
- *Hop at `v_from` only* (instantaneous contacts). Loses interval semantics: a fact valid `[0, 10)` could not be used at 5.

### Decision 2: `REACH` is label-correcting and emits after the search

Plain BFS with a visited set is wrong: a longer walk can arrive earlier (`A→C [10,20)` direct, `A→B [1,2)`, `B→C [1,3)` gives arrival 10 at 1 hop and 1 at 2 hops), and a state first reached with a late τ may need re-expansion with an earlier one to reach further. The search keeps `best[(node, state)]`, the smallest τ seen; a state reached again with a strictly smaller τ is pushed into the current layer again. Feasibility is monotone (a smaller τ allows every hop a larger one allows and yields a τ' no larger), so:

- **Arrival is minimal.** By induction on the length k of a time-respecting walk to `(v, q)` with arrival τ: its prefix reaches `(u, p)` with τ_p, and some layer j ≤ k−1 set `best[(u, p)] ≤ τ_p` and expanded it; the last hop is feasible from that smaller label and gives a τ' ≤ τ at layer j+1 ≤ k. So when the search ends (empty frontier or hop bound), `best` holds the minimum over every walk within the bound.
- **Hops are minimal.** The same induction shows the end is first reached no later than the layer of its shortest time-respecting walk, and it cannot be reached earlier than that; the first layer that reaches it is recorded.
- **Termination.** τ only takes values from the start τ and the `v_from` of statements, and each re-push strictly lowers a state's τ, so a state is pushed at most (distinct τ values + 1) times; cycles terminate without a hop bound.

Because a later layer can still lower an arrival, rows are emitted only when the search is finished, ordered by (hops, raw id) as today. A `WHERE "end" = ?` pushdown filters the rows but cannot stop the search early. Emitting at the first layer would give the right ends and hops but not provably minimal arrivals, so it is not done.

### Decision 3: `TRAIL` prunes per path

Each arena entry of a trail carries its τ; a hop that is infeasible under that τ is not taken. Every row is one time-respecting trail and reports that trail's arrival. Trails are enumerated exactly as today otherwise (identity check, hop-key order).

### Decision 4: Shortest modes with Pareto pruning per layer

A label is `(node, state, τ)` at a layer k. A label is kept only if its τ is strictly smaller than the best τ of `(node, state)` in every earlier layer; labels of the same layer with different τ are all kept.

- **Pruning is safe.** If `(x, q)` was reached at layer j < k with τ_j ≤ τ_k, any continuation of the layer-k label is feasible from the layer-j label (monotonicity) and ends k−j hops earlier, so the layer-k label lies on no shortest time-respecting path.
- **Nothing shortest is lost.** On a shortest walk to an end, no prefix label is dominated (otherwise cutting would give a shorter walk), and labels of equal key in a layer share predecessors, so every shortest time-respecting walk is a path of the layered DAG. A shortest walk never repeats `(node, state)` (the later visit would be dominated), so these are paths, as in the non-temporal modes.
- **Order.** Labels are created in the order of their parent in the layer and then hop key, so the first label of an end is the lexicographically smallest shortest path (`ANY_SHORTEST`), and `ALL_SHORTEST` enumerates the DAG backwards and sorts by hop key, as today.
- **Arrival** of a row is its label's τ; two shortest paths to one end may arrive at different times.

The number of labels per `(node, state)` is bounded by the distinct τ values, and the search-state guard counts labels.

### Decision 5: `arrival: Option<i64>`

`None` for a request that is not time-respecting. For one that is, `None` means −∞: no `after` was given and no traversed statement had a `v_from` (the journey is possible "since always"); otherwise the instant in epoch ms. A dedicated enum (`NotTemporal | Unbounded | At(i64)`) was considered; `Option` keeps `PathRow` simple, and a caller knows whether it asked for a time-respecting search. `tm_path` returns the same as an INTEGER or NULL, and the bridge as a number or `null`.

### Decision 6: `tm_path` takes the option in the view text

`tm_path` already has a free-form `view` argument, and time respect is a property of how hops read time, so the view text gains a `timeRespecting` part: `timeRespecting` (from −∞) or `timeRespecting/<t>` with `t` an RFC 3339 date or date-time, or an integer of epoch milliseconds (unlike `validAt`, whose instant is a date). It combines with the other parts in any order, at most once, also with the `urn:tiramemsu:tm:` prefix. A new argument would have been a seventh positional argument after `graphs`; the view text keeps the call short and reads as one time specification. The output gains a fifth column `arrival` after `path_json`, so positional readers of the first four columns are unaffected; `SELECT *` now yields five columns.

### Decision 7: The fetch reads the interval always

The fetch statements select `t.v_from, t.v_to` for every shape. Both columns are in every covering index and in the row read for a rowid lookup, so no plan changes (the fetch plan snapshots are unchanged; the SQL-text snapshot of the graph filter gains the two columns); a non-temporal search ignores them.

## Risks / Trade-offs

- **`REACH` cannot stop early** under time respect, so `ASK`-like uses (`WHERE "end" = ?`) run the whole search. Bounded by the state guard; an early stop when the frontier's smallest τ is not below the target's arrival is a possible optimisation.
- **Label growth** in shortest modes and re-expansion in `REACH` depend on the number of distinct `v_from` values; the state guard still fails with `PathLimitExceeded` rather than truncate.
- **`validAt` plus time respect** is legal and rarely meaningful; documented rather than rejected.

## Migration Plan

Additive. Existing calls are unchanged except that `tm_path` has one more output column and `PathRow` one more field (`arrival: None` for existing requests). Code that builds `PathRow` literals must add the field.

## Open Questions

- **SPARQL and Cypher syntax.** A SPARQL `SERVICE <urn:tiramemsu:tm:timeRespecting/…>` scope around a path is the natural spelling, since `SERVICE` already carries time; Cypher could take a path-pattern prefix like `TIME RESPECTING` or a `USE` clause. Either needs the IR `PathPattern` to carry the option and the planner to add it to the view text. Not in this change.
- Waiting-time limits, latest departure and fastest journeys.
