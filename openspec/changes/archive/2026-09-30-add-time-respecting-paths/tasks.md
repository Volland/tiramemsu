## Dependencies

- `add-path-engine` (archived) and `add-graph-scoped-paths` (the `PathArgs` struct and the graph set, which combines with this option).
- Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. The requirement text is in `specs/*/spec.md` of this change.

## 1. Engine

- [x] 1.1 Add `TimeRespecting { after: Option<i64> }` and `PathRequest.time_respecting`; add `PathRow.arrival: Option<i64>` (None for non-temporal requests) and update every `PathRow` and `PathRequest` literal.
- [x] 1.2 Make the fetcher return `v_from` and `v_to` with each neighbour (always selected; check that no fetch plan changes).
- [x] 1.3 The hop rule (design Decision 1) as one function shared by the modes.
- [x] 1.4 `REACH`: the label-correcting search that emits after finishing (design Decision 2).
- [x] 1.5 `TRAIL`: τ per arena entry, infeasible hops pruned (Decision 3).
- [x] 1.6 `ANY_SHORTEST` and `ALL_SHORTEST`: labels `(node, state, τ)` with Pareto pruning per layer (Decision 4), or `Unsupported("time-respecting shortest paths")` if that cannot be made correct.

> Notes (group 1): `TimeRespecting` lives in `tm_exec::path::engine` (re-exported by `tm_exec` and the facade) with a doctest. The search context carries `time: Option<i64>` (`i64::MIN` is −∞) and `Ctx::arrival` turns a final time into the row's `arrival`. The hop rule is `Nb::step_time` in `path/fetch.rs`. `search` dispatches `REACH` to `reach::run_timed` and the shortest modes to `shortest::run_timed` when the option is set; `TRAIL` shares `trail::run` (entries carry `tau`). Shortest modes are implemented, not `Unsupported`: the brute-force property test covers them. The fetch statements now select `t.v_from, t.v_to` for every shape: `path_fetch__every_shape_uses_an_index_or_rowid.snap` and `path_graphs__graph_filter_plans.snap` are unchanged, only the SQL-text snapshot `path_graphs__graph_filter_sql.snap` gained the two columns.

## 2. Tests of the engine

- [x] 2.1 A model-based property test: for random small graphs with random intervals and random `after`, the `REACH` ends, hop counts and minimal arrivals equal a brute-force enumeration of every time-respecting walk up to the hop bound; `TRAIL` rows equal the time-respecting edge-distinct walks with their arrivals; `ALL_SHORTEST` rows equal the minimal-length time-respecting walks, and `ANY_SHORTEST` one of them per end.
- [x] 2.2 Hand scenarios of "Time-respecting evaluation": the infection chain, same-instant chaining and its boundary, `after` cutting early facts, unbounded intervals, a longer walk arriving earlier, shortest paths, the combination with a graph set, virtual hops through layers, and unchanged results without the option.

> Notes (group 2): `crates/tm-exec/tests/path_time_respecting.rs`. The property test (256 cases, 2 to 9 edges over 4 nodes, intervals from `0..6` with lengths `1..5` or unbounded, `after` in `0..7` or none, five path expressions, hop bound 4) checks every mode, and `ANY_SHORTEST` against the lexicographically smallest minimal walk, not just "one of them". Mutation check while writing it: replacing the label-correcting test in `reach::run_timed` or the Pareto test in `shortest::run_timed` by a plain visited set makes it fail (with 1 to 6 edges the shortest-mode mutation went unnoticed, hence 2 to 9 edges and the explicit "re-expanded with an earlier time" hand scenario); the shrunk failing case is kept in `path_time_respecting.proptest-regressions`. The worked example of the task, `B→C [0,2)` after arriving at B at 1, is reachable with arrival 1; `[0,1)` is not.

## 3. tm_path

- [x] 3.1 Parse `timeRespecting` and `timeRespecting/<t>` in the view text (RFC 3339 or epoch ms, any order, at most once, `tm:` prefix).
- [x] 3.2 Add the output column `arrival` after `path_json`.
- [x] 3.3 Tests: view forms and errors, the `arrival` column with and without the option, `SELECT *` columns.

> Notes (group 3): `path::view::parse_view_arg` returns `(ViewSpec, Option<TimeRespecting>)`; `parse_view` keeps its signature and rejects a `timeRespecting` part. Epoch milliseconds may be negative. `path_sql.rs` `callable_everywhere_and_read_only` now expects five columns. Tests: `view.rs` `time_respecting_view_parts`, `path_time_respecting.rs` `tm_path_arrival_column`.

## 4. Facade and bridge

- [x] 4.1 `PathArgs.time_respecting` and the re-export of `TimeRespecting`; doctest.
- [x] 4.2 JSON bridge `path`: `timeRespecting: true | {"after": <time>}`, `arrival` in every row; test.

> Notes (group 4): the `View::path_with` doctest now also runs a contact chain. The bridge reads `after` with the bridge's time encoding (`time_from_json`: epoch ms or an RFC 3339 string); `false` and `null` mean no option; any other value, or an object key other than `after`, is `InvalidArgument`. Every `path` row now has an `arrival` key. The Node and Python wrappers are not changed (out of scope for this change): they pass `path` arguments through the bridge but do not yet expose `graphs`, `timeRespecting` or `arrival` in their typed APIs.

## 5. Documentation

- [x] 5.1 Update `lat.md/query.md` (Path Engine: move time-respecting paths out of "Later", tm_path view text and output) and `lat.md/time-model.md#Valid Time`.
- [x] 5.2 Add `lat.md/tests.md` sections for the new tests and run `lat check`.

> Notes (group 5): `lat.md/query.md` gains the section Path Engine › Time-Respecting Search; SPARQL and Cypher syntax stays under "Later". Also `lat.md/api.md` and `lat.md/bindings.md`. Five Query leaves in `tests.md`; `lat check` passes.
