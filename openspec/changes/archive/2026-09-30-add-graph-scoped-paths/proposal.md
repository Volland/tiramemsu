## Why

Named graphs (`add-named-graphs`, archived) left one hole: a recursive property path under `GRAPH <g>`, `GRAPH ?g` or a `FROM <g>` default graph fails with `Unsupported("named graph path")`, because the path engine takes a view and has no graph filter (`named-graphs` "Unsupported combinations fail before execution"). Agent memory groups statements by session or source document, and "who can alice reach through what this session recorded" is exactly a path inside a graph. Today the only workaround is to run the path over the union and filter hop by hop by hand in SQL, which a recursive path cannot express.

Graphs are tags on statements (D28), so a graph filter is a condition on each traversed statement: its eid must have a visible `sys:inGraph` membership. That fits the neighbour fetcher's one batched statement per hop shape as one more `EXISTS` on an index seek, without a new operator or a change to the search.

## What Changes

- **Path engine:** a request may carry a graph set G. A path is in G when every statement it traverses is a member of at least one graph of G, the membership being visible in the same view as the hop. This applies to stored hops (the statement traversed), to the virtual hops `sys:subject`, `sys:object`, `sys:predicate` and their inverses (the statement whose part is stepped to or from), and to wildcard hops. Zero-hop rows are unaffected. `PathRequest` gains `graphs: Option<Vec<ObjectId>>`.
- **IR:** `PathPattern` gains a graph selector (`GraphSel`, default `Any`), printed and validated like the selector of `TriplePattern`. Every existing plan is unchanged.
- **`tm_path`:** a sixth optional argument `graphs`: NULL (no filter), an INTEGER ObjectId of one graph, or TEXT holding a JSON array of integer ids. Errors start with `tm_path: graphs:`.
- **Planner:** `Set` passes its encoded ids; `Var(g)` passes the column of `g` when a pattern joined with the path binds it, and otherwise enumerates the graphs visible in the view and runs the path once per graph, binding `g`.
- **SPARQL:** property paths inside `GRAPH <g>`, `GRAPH ?g` and under `FROM <g>` defaults run on the path engine with the graph selector of their block. The `Unsupported("named graph path")` rejection is removed. A `GRAPH ?g` block whose only pattern is a path is accepted.
- **Facade:** `View::path_with(start, path, &PathArgs)` with `PathArgs { mode, max_hops, graphs, … }` and `Default`; `View::path` stays as the shorthand.
- **JSON bridge:** the `path` operation accepts an optional `graphs` list of terms.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `path-evaluation`: adds graph-scoped evaluation.
- `path-table-function`: `tm_path` gains the `graphs` argument and its errors; `View::path_with` and `PathArgs`.
- `path-lowering`: SPARQL paths inside named graphs.
- `named-graphs`: property paths are no longer an unsupported combination; `USE GRAPH` still is.
- `sparql-query`: the out-of-subset list no longer names paths inside graphs.
- `sparql-update`: property paths in an update `WHERE` inside `GRAPH` or under `USING` run with the graph set.
- `json-bridge`: the `path` operation takes `graphs`.

## Impact

- **`tm-exec`:** `path/fetch.rs` (the membership `EXISTS`), `path/engine.rs` (`PathRequest.graphs`), `path/vtab.rs` (argument), `plan/normalize.rs`, `plan/mod.rs` (`PPath`), `sqlgen/native.rs` (argument and graph enumeration), `plan/bind.rs` and `plan/route.rs`.
- **`tm-ir`:** `PathPattern.graph`, its text form and validation. Front ends that build `PathPattern` literals (Cypher) set `GraphSel::Any`.
- **`tm-sparql`:** `lower/pattern.rs` and `lower/path.rs`; `error.rs` loses `NAMED_GRAPH_PATH`.
- **Facade and bridge:** `View::path_with`, `PathArgs`; `bindings/json` `path`.
- **Not affected:** storage format, indexes, Cypher syntax (Cypher keeps one graph), the Node and Python binding sources (they pass the bridge arguments through unchanged).
- **Docs:** `lat.md/query.md` (Path Engine, tm_path, Path Lowering, SPARQL named graphs), `lat.md/data-model.md#Named Graphs`, `lat.md/tests.md`.
