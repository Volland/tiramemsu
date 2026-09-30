## Why

Every statement has a valid interval `[v_from, v_to)`, and the path engine can already restrict all hops to one instant (`validAt`). What it cannot answer is the temporal-graph question "could something travel from A to C along these facts, in time order": an infection passed on by contacts, a message forwarded through a chain of people, an ownership chain where each transfer happens after the previous one. Such a journey uses each fact at some instant inside its interval, and the instants never go backwards. A `validAt` view fixes one instant for every hop, and a plain path ignores order, so both give wrong answers. `lat.md/query.md` lists time-respecting paths under Path Engine "Later"; this change builds them for the engine API, `tm_path` and the JSON bridge.

## What Changes

- **Semantics (earliest arrival).** A time-respecting search carries a time τ, starting at `after` (epoch ms) or at −∞. A stored hop over a statement with `[v_from, v_to)` (NULL is unbounded) is allowed when `v_to` is NULL or `v_to > τ`; afterwards τ becomes `max(τ, v_from)` (a NULL `v_from` leaves τ unchanged). τ never decreases, so hops at the same instant chain. Virtual hops (`sys:subject`, `sys:object`, `sys:predicate` and inverses) are structural: always allowed, τ unchanged. The view's own `validAt`, if any, still filters hops independently, and a graph set (`add-graph-scoped-paths`) still applies.
- **Modes.** `REACH` returns each end reachable by some time-respecting walk once, with the shortest such walk's hop count and the earliest arrival over all such walks (a label-correcting search that finishes before it emits). `TRAIL` prunes infeasible hops per path and reports each path's arrival. `ANY_SHORTEST` and `ALL_SHORTEST` return the shortest time-respecting paths per end, with Pareto pruning per BFS layer.
- **Rows.** `PathRow` gains `arrival: Option<i64>`: `None` for a request that is not time-respecting, and, for one that is, `None` when no bound applies (no `after` and no traversed statement had a `v_from`), otherwise the arrival instant in epoch ms.
- **API.** `PathRequest.time_respecting: Option<TimeRespecting>` with `TimeRespecting { after: Option<i64> }`; the facade's `PathArgs.time_respecting`.
- **`tm_path`.** The `view` text accepts a `timeRespecting` or `timeRespecting/<RFC 3339 or epoch ms>` part, combinable with the other parts (`now;validAt/2025-01-01;timeRespecting/2024-06-01`), and the function gains a fifth output column `arrival` (INTEGER ms or NULL). The four existing columns are unchanged.
- **JSON bridge.** `path` accepts `timeRespecting: true | {"after": <time>}`, and every row carries `arrival` (a number or null).
- **Not in this change:** SPARQL and Cypher syntax for time-respecting paths (design.md, Open Questions).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `path-evaluation`: adds time-respecting evaluation and the `arrival` of a result row.
- `path-table-function`: the `timeRespecting` view part, the `arrival` output column, and `PathArgs.time_respecting`.
- `json-bridge`: the `path` operation takes `timeRespecting` and returns `arrival`.

## Impact

- **`tm-exec`:** `path/fetch.rs` (the neighbour carries `v_from`, `v_to`), `path/search/*` (the time-respecting searches), `path/engine.rs` (`TimeRespecting`, `PathRequest.time_respecting`), `path/row.rs` (`PathRow.arrival`), `path/view.rs` and `path/vtab.rs` (`timeRespecting`, `arrival`).
- **Facade and bridge:** `PathArgs.time_respecting`, the re-export of `TimeRespecting`; `bindings/json` `path`.
- **Not affected:** storage and indexes (the covering indexes already hold `v_from`, `v_to`), the IR, SPARQL and Cypher, the planner's `tm_path` calls (they never pass `timeRespecting`), the Node and Python binding sources.
- **Depends on** `add-graph-scoped-paths` (the `PathArgs` struct and the `graphs` combination).
