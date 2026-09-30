## Context

- **Graphs are tags (D28).** A statement `e` is a member of graph `g` in a view when `e` is visible and a membership `(m, e, sys:inGraph, g)` is visible in the same view (`named-graphs` "Graph and membership model"). The triple-pattern selector lowers to a membership join in `tm-exec` `plan/bind.rs`.
- **The path engine reads one view.** `PathEngine::run` resolves the view once, and `Fetcher` builds one prepared statement per hop shape (`Pred`, `Any`, `Virt(kind)` × direction) of the form `SELECT … FROM rarray(?1) AS r CROSS JOIN triple AS t WHERE <shape> AND <view predicates on t>`. The SQL text holds no data; every id is a parameter.
- **Virtual hops read the statement row.** `sys:subject` out from a statement `e` reads row `e` by rowid; `sys:subject` in from a node `n` reads the rows with `s = n`. In both cases the row `t` is the statement whose part is stepped to or from.
- **Zero-length rule (Path Lowering).** A nullable path from a constant that is in no statement binds the far end to that very term; the normaliser emits a `VALUES` row instead of a `tm_path` call.
- **SPARQL lowering.** `GRAPH <g>` sets `GraphSel::Set([g])` on the block's triple patterns, `GRAPH ?g` sets `GraphSel::Var(?~gN)` (an internal variable per block, bound to the user variable afterwards), and `FROM <g1> FROM <g2>` sets `Set([g1, g2])` on the default graph. Paths under any of these are rejected today.

## Goals / Non-Goals

**Goals:**
- Recursive property paths work under `GRAPH <g>`, `GRAPH ?g` and `FROM <g>` with the membership semantics of triple patterns, including time travel of memberships.
- One code path: the engine, `tm_path`, the planner and `View::path_with` share the filter.
- The fetch stays one batched statement per hop shape and an index seek per frontier node.
- Existing plans, SQL and snapshots are unchanged when no graph is selected.

**Non-Goals:**
- Cypher graph selection (`USE GRAPH` stays `Unsupported`).
- Graph filters on the non-recursive SPARQL translation beyond what triple patterns already do (see Decision 6).
- Per-hop different graphs (a path that may switch graphs). A path is in G as a whole; `GRAPH ?g` binds one graph per path.

## Decisions

### Decision 1: Membership is an `EXISTS` on each traversed statement

With a graph set G, every fetch shape gets one more condition on its row `t`:

```sql
EXISTS (SELECT 1 FROM triple AS m
        WHERE m.s = t.eid AND m.p = ?ig AND m.o IN (SELECT value FROM rarray(?gs))
          AND <view predicates on m>)
```

`?ig` is the id of `sys:inGraph` and `?gs` the graph ids; both are parameters, so the SQL text stays data-free and one statement per shape is prepared. The membership is read under the same view function as the hop, so `asOf` shows a path as it was in the graph and `validAt` honours bounded memberships. Because `t` is the statement row for every shape, stored, virtual and wildcard hops share the rule: a virtual hop is allowed when the statement whose part it steps to or from is in G.

Alternatives considered:
- *Join the membership row in the fetch* (`JOIN triple m ON …`). A statement in two graphs of G would come back twice and the search would see duplicate neighbours; `EXISTS` keeps it once, which is what the triple-pattern `Set` lowering also does.
- *Filter in Rust after the fetch* (read the eids' memberships in a second batch). Two round trips per layer and a second prepared statement; no gain, since the seek is on the covering `(s, p, o)` index either way.
- *A precomputed member set.* Reading every member of G up front costs the size of the graph even for a two-hop query.

Plan: every column of the membership lookup is bound (`s = t.eid`, `p = ?ig`, `o` from the list), so it is a covering-index seek under every view; see the recorded plans in Risks.

### Decision 2: `graphs: Option<Vec<ObjectId>>` on the request

`None` means no filter (every existing caller). `Some(vec![])` and a set whose ids have no membership are legal and mean "no statement is in G": the search returns only zero-hop rows. If `sys:inGraph` is not in the dictionary, the same holds without running any fetch. The engine does not check that the ids are graph names: an id that names no graph matches no membership.

### Decision 3: The IR selector is `GraphSel` on `PathPattern`

`PathPattern.graph: GraphSel`, default `Any`, reusing the triple selector with the same validation (a `Set` is non-empty and holds constants and parameters only) and the same text form (`:graph (<g1> <g2>)` or `:graph ?g`). `Var(g)` is in the pattern's scope. Parameters in a `Set` are bound by `plan/bind.rs` like the endpoints.

### Decision 4: `GRAPH ?g` correlates or enumerates

For `GraphSel::Var(g)` the SQL generator looks at the patterns already compiled into the same join (paths are compiled after ordinary patterns):

- **Correlate:** a pattern of the join binds `g` (in SPARQL, a triple pattern of the same `GRAPH ?g` block). The call gets `g`'s column as its `graphs` argument, so each row's path is in that row's graph.
- **Enumerate:** otherwise the generator adds `(SELECT DISTINCT gm.o AS g FROM triple AS gm WHERE gm.p = ?ig AND <view gm> AND EXISTS (SELECT 1 FROM triple AS ge WHERE ge.eid = gm.s AND <view ge>)) AS gN` before the call and passes `gN.g`: the graphs visible in the view (objects of visible memberships of visible statements), each giving its own call. `g` is bound to the enumerated graph.

The enumeration costs one scan of the visible memberships (`sys:inGraph` seek on `(p, o)`), like `GRAPH ?g { ?s ?p ?o }` does today. It is done in SQL generation rather than as an IR rewrite because only the generator knows which variables the join has bound.

### Decision 5: Zero-length paths per graph in scope

A path with no hop traverses no statement, so it is in every graph. The rule of Path Lowering is applied per graph in scope:

- `GRAPH <g> { :a :p* ?x }` and `FROM <g1> FROM <g2>` (one default graph) give `?x = :a` once, even when `:a` is in no statement and whether or not `<g>` has members. A `FROM NAMED` list that excludes `<g>` still empties the block (existing rule).
- `GRAPH ?g { :a :p* ?x }` gives `?x = :a` once per graph visible in the view (enumerated) or once per bound `?g` (correlated), because the block's solutions are per graph.
- For a constant that is in no statement under `GRAPH ?g`, the normaliser cannot emit a `VALUES` row (it would lose `?g`), so it calls `tm_path` from the constant's plan-local id: the engine finds no neighbour and returns the zero-hop row, once per graph, and the decoder maps the plan-local id back to the term.

### Decision 6: Non-recursive paths keep the triple translation

A path of only `/`, `|` and `^` is still the SPARQL 1.1 translation to triple patterns, now with the block's selector on each triple (like a BGP). A virtual-predicate step in such a path (`:b :supportedBy/sys:subject ?x`) is a virtual-predicate pattern and reads with no selector, exactly as the same text written as a BGP does (spargebra already desugars `a/b` of IRIs into a BGP). A recursive path filters every hop, virtual ones included. The asymmetry is documented in `lat.md/query.md`; making the BGP translation filter virtual steps would change existing BGP semantics.

### Decision 7: `tm_path(…, graphs)` argument encoding

`graphs` is the sixth argument, so SQL can correlate it with a column: NULL is no filter, an INTEGER is one graph, TEXT is a JSON array of integers (`'[12,40]'`, `'[]'` for the empty set). Anything else (a REAL, a BLOB, malformed JSON, a non-integer element) fails with `tm_path: graphs: …`. The generator passes one graph as an INTEGER parameter and several as a TEXT parameter. The pushed-down `"end"` moves from argument index 5 to 6; the host appends pushdowns after the declared arguments, so nothing else changes.

### Decision 8: One argument struct on the facade

`View::path_with(start, path, &PathArgs)` takes `PathArgs { mode, max_hops, graphs }` with `Default` (`REACH`, `u32::MAX`, no filter). `add-time-respecting-paths` adds its option to the same struct. `View::path(start, path, mode, max_hops)` is unchanged and calls `path_with`.

## Risks / Trade-offs

- **One more seek per neighbour and graph.** Recorded plans (test `graph_filter_uses_an_index_seek`, snapshot `path_graphs__graph_filter_plans.snap`, after `ANALYZE` on a store with 200 memberships over three graphs): SQLite turns `m.o IN (SELECT value FROM rarray(?))` into one equality seek per listed graph on a covering index with the whole key bound.
  - `Now`, `pred out`: `SCAN r VIRTUAL TABLE INDEX 1: | SEARCH t USING COVERING INDEX hist_spo (s=? AND p=?) | SEARCH m EXISTS USING COVERING INDEX live_osp (o=? AND s=? AND p=? AND t_ret=?) | LIST SUBQUERY 1 | SCAN rarray VIRTUAL TABLE INDEX 1:`; the virtual `subject out` shape seeks `live_spo (s=? AND p=? AND o=? AND t_ret=?)` for `m`.
  - `asOf`: `SEARCH m EXISTS USING COVERING INDEX hist_osp (o=? AND s=? AND p=? AND t_add<?)` (`hist_spo` for `subject out`).
  - `history`: `SEARCH m EXISTS USING COVERING INDEX hist_pos (p=? AND o=? AND s=?)` (`hist_spo` for `subject out`).
  - No shape scans `t` or `m`; the cost is one seek per (neighbour, listed graph), independent of the graph sizes. No new index is needed.
- **Enumeration of `GRAPH ?g`** is proportional to the visible memberships. A block that also has a triple pattern correlates instead; the docs say so.
- **Semantics differ from W3C for graphs that do not exist.** `GRAPH <g> { :a :p* ?x }` gives `:a` even when `<g>` has no member (Decision 5); W3C evaluates `GRAPH <g>` to nothing when `g` is not a named graph of the dataset. The union default graph (D28) already departs from W3C datasets, and the W3C runner lists any test this changes.

## Migration Plan

Additive. Queries that failed with `Unsupported("named graph path")` now run. `tm_path` calls with five arguments are unchanged. No format change.

## Open Questions

- Cypher `USE GRAPH` could reuse the selector later.
- A path that may switch graphs per hop (`GRAPH ?g` bound per hop) is not expressible in SPARQL and is not planned.
