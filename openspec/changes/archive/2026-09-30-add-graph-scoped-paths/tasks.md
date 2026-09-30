## Dependencies

- `add-path-engine` and `add-named-graphs` are archived and are the base of this change. `add-time-respecting-paths` builds on it (it adds its option to the same `PathArgs`).
- Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. The requirement text is in `specs/*/spec.md` of this change.

## 1. Engine

- [x] 1.1 Add `graphs: Option<Vec<ObjectId>>` to `PathRequest` and pass it to the `Fetcher`. Update every `PathRequest` literal (facade, tests, benches).
- [x] 1.2 In `Fetcher::build`, add the membership `EXISTS` on `t.eid` with `sys:inGraph` and the graph ids as parameters and the view predicates on `m`, for every hop shape. Resolve `sys:inGraph` once per fetcher; when it is missing or the set is empty, fetch nothing.
- [x] 1.3 Tests: a snapshot of the SQL of every hop shape with graphs (no data in the text), `EXPLAIN QUERY PLAN` shows an index seek for `m` under `Now` and `asOf` (record the plan in design.md, Risks), and engine scenarios of "Graph-scoped evaluation" (confined path, two graphs one hop, asOf of a removed membership, virtual hops, no membership predicate).

> Notes (group 1): `Fetcher::with_graphs(Option<&[ObjectId]>) -> Result<Fetcher>` resolves `sys:inGraph` once and keeps a private `Scope` (`All`, `Graphs { in_graph, ids }`, `Nothing`); `Nothing` (empty set or no `sys:inGraph` in the dictionary) returns before any SQL. The graph ids are sorted and deduplicated and bound as a second `rarray` parameter. The plans were not the ones design.md first guessed: SQLite expands the `IN (SELECT value FROM rarray(?))` into one seek per listed graph with the whole key bound (`live_osp (o=? AND s=? AND p=? AND t_ret=?)` under `Now`, `hist_osp` under `asOf`, `hist_pos` under `history`, `*_spo` for the rowid-driven virtual shapes); design.md Risks now records the real plans. Tests: `crates/tm-exec/tests/path_graphs.rs` (`graph_scoped_paths_follow_membership`, `virtual_hops_need_the_stepped_statement_in_the_graph`, `graph_filter_uses_an_index_seek` with the snapshots `path_graphs__graph_filter_sql` and `path_graphs__graph_filter_plans`).

## 2. tm_path

- [x] 2.1 Add the sixth argument `graphs` (NULL, INTEGER, TEXT JSON array) with `tm_path: graphs:` errors; the pushed-down `"end"` moves to index 6.
- [x] 2.2 Tests: integer, JSON text, correlated column, and each malformed form.

> Notes (group 2): the JSON array is parsed by hand (`[`, comma-separated integers, `]`), because `tm-exec` has no JSON dependency; whitespace is allowed, `'[]'` is the empty set. A REAL, a BLOB, text that is not an array and a non-integer element fail with `tm_path: graphs: expected NULL, an integer ObjectId or a JSON array of them`. Test `tm_path_graphs_argument` also checks that the pushed-down `WHERE "end" = ?` still stops the search.

## 3. IR and planner

- [x] 3.1 Add `graph: GraphSel` to `PathPattern` (default `Any` in the builder and every literal), with the text form `:graph`, scope of `Var`, and the validation of `Set`. Round-trip and validation tests.
- [x] 3.2 Bind parameters in a path's `Set` in `plan/bind.rs`.
- [x] 3.3 Plan the selector: `Set` encodes its graphs (unknown ones dropped) into `PPath`; `Var(g)` is kept as a variable; the view and the id of `sys:inGraph` go with it.
- [x] 3.4 Generate the call: one graph as an INTEGER parameter, several as a JSON TEXT parameter; `Var(g)` correlates with a column of the join when one binds `g`, otherwise a graph-enumeration subquery is added before the call and binds `g`.
- [x] 3.5 Zero-length rule per graph: `Any`/`Set` keep the `VALUES` row; `Var` calls `tm_path` from the constant's plan-local id.
- [x] 3.6 Tests: SQL snapshots of the call with `Set` of one, of several, correlated `Var` and enumerated `Var`.

> Notes (group 3): `PathPattern::in_graph` mirrors `TriplePattern::in_graph`; the display helper `graph_text` and the validation helper `check_graph` are now shared by both patterns. The IR snapshot of `tm-ir` is unchanged (`Any` prints nothing). `PPath` gained `graphs: PGraphs` (`Any`, `Ids`, `Var`), `view` and `in_graph`; a path without a selector still generates the five-argument call, so every existing SQL snapshot is unchanged. `route::bound_before` counts a path's graph variable as bound once one of its endpoints is. The enumeration is `(SELECT DISTINCT gm.o AS g FROM triple AS gm WHERE gm.p = ? AND <view gm> AND EXISTS (SELECT 1 FROM triple AS ge WHERE ge.eid = gm.s AND <view ge>)) AS xN`, and `(SELECT NULL AS g WHERE 0)` when `sys:inGraph` was never written. Tests: `tm-ir` `path_graph_selector_text_and_validation`; `tm-exec` `path_graph_selector_sql_snapshots` (snapshots `path_graph_selector@{set_one,set_several,var_enumerated,var_correlated}`).

## 4. SPARQL

- [x] 4.1 Pass the active graph selection to recursive paths and to the triples of the non-recursive translation; count a path as a use of `GRAPH ?g`. Remove `NAMED_GRAPH_PATH`.
- [x] 4.2 Tests: `GRAPH <g>`, `GRAPH ?g` unbound and bound, `FROM`, `FROM NAMED` exclusion, asOf of a removed membership, virtual hops through layers inside a graph, the zero-length choice, and the IR text of each lowering.
- [x] 4.3 Run the W3C subset; update `expected-deviations.toml` if a test changes status and record the counts.

> Notes (group 4): the non-recursive translation calls `Lowerer::select_graph` (now `pub(super)`) on each triple, so a virtual-predicate step reads without a selector exactly as in a BGP (design Decision 6). `GRAPH_WITHOUT_PATTERN` now fires only for a block with neither a triple pattern nor a recursive path. The crate README's unsupported list no longer names paths inside graphs. Tests: `crates/tiramemsu/tests/sparql_graphs.rs` (`property_paths_run_inside_graphs`, `zero_length_paths_per_graph`, `graph_paths_under_as_of`, `paths_cross_layers_inside_graphs`, replacing `property_paths_inside_graphs_are_rejected`) and `crates/tm-sparql/tests/lower_patterns.rs` (IR text). W3C: 634 passed, 147 failed (147 expected) before and after; no test changed status, so `expected-deviations.toml` is unchanged (the suite has no recursive path inside `GRAPH` or under `FROM`).

## 5. Facade and bridge

- [x] 5.1 Add `PathArgs` and `View::path_with`, re-export `PathArgs`, keep `View::path` as the shorthand. Doctest.
- [x] 5.2 JSON bridge `path`: optional `graphs` list of terms; test.

> Notes (group 5): `PathArgs { mode, max_hops, graphs }` with `Default` (`REACH`, `u32::MAX`, `None`) lives in `crates/tiramemsu/src/view.rs`; `View::path` builds one and calls `path_with`. The bridge encodes each listed term with `View::encode` and drops terms that are not stored (they name no graph), so a list of unknown graphs gives only zero-hop rows; a non-list `graphs` is `InvalidArgument`. Test `paths_stay_inside_the_listed_graphs` in `bindings/json/tests/bridge.rs`.

## 6. Documentation

- [x] 6.1 Update `lat.md/query.md` (Path Engine, tm_path, Path Lowering, SPARQL named graphs) and `lat.md/data-model.md#Named Graphs`.
- [x] 6.2 Replace `lat.md/tests.md#Named Graphs#Property Paths Inside Graphs Are Rejected` with the new behaviour, add sections for the new tests, and run `lat check`.

> Notes (group 6): also `lat.md/api.md` (`View::path_with`) and `lat.md/bindings.md` (the `graphs` argument). `tests.md` gains ten Named Graphs leaves (`Property Paths Run Inside Graphs` replaces the rejected one); `lat check` passes.
