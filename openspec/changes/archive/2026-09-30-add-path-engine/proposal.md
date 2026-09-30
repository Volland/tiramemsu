## Why

Agent memory is navigated by paths. Typical questions are "who is reachable from Alice over `knows`", "the shortest chain between two entities", and "which entities stand behind the statement this belief relies on". M1 compiles triple patterns to SQL, but SQL can only express these as recursive CTEs, and those cannot give trail or shortest-path semantics or path values in a practical way (`lat.md/query#Physical Planning`, decision D12). Decision D15 (`lat.md/query#Physical Planning#Path Engine`) calls for a native, time-aware path operator that serves SPARQL, Cypher, the Rust API and SQL. It is milestone M3 (`lat.md/roadmap#Milestones`). Now that the IR and SQL planner exist, the front ends need it to finish their v1 surface.

## What Changes

- Add a native **path operator** to `tm-exec`. It compiles a path expression (`/ | * + ? ^`, bounded repetition, alternation of predicates, directions) to an NFA and runs a breadth-first search over the product of the NFA and the graph. It is never a recursive CTE.
- Support the v1 **path modes**:
  - `REACH`: SPARQL reachability. Endpoints only, set semantics, zero-length matches for `*` and `?`.
  - `TRAIL`: Cypher. No relationship eid repeats within a path.
  - `ANY_SHORTEST`: one shortest path per reachable end.
  - `ALL_SHORTEST`: every shortest path per reachable end.
- Enforce the **limits**. At least one endpoint must be bound, or the query fails. An unbounded Cypher pattern is capped at 15 hops by default and stops at the cap without failing. Callers set a `max_hops` bound. Cycles always terminate. A search-state guard stops runaway memory use.
- Make paths **time-aware**. Every hop reads through the same view as the pattern (`Now`, `AsOf(t)`, `History`, optionally `ValidAt(d)`), and uses the same view-predicate function as scans (`lat.md/query#Views and Scans`).
- Add **virtual layer hops**: `sys:subject`, `sys:object` and `sys:predicate`, plus their inverses. They step between a statement and its parts, so a path can cross statement layers (`lat.md/data-model#Layers`).
- Return **path values**: the ordered node sequence and hop sequence, including each hop's eid, predicate and direction. Ordering is deterministic where specified.
- Add the **`tm_path` table-valued function**. It is an eponymous virtual table `tm_path(start, path, mode, max_hops, view)` that returns `(start, end, hops, path_json)` and is registered on every connection. It composes with the SQL regions the planner generates.
- Add the **`View::path`** API entry (`lat.md/api#Rust Surface`), backed by the same operator.
- **Lower SPARQL property paths** and **Cypher variable-length, `shortestPath` and `allShortestPaths` patterns** to the path operator through the planner's table-valued-function region hook. This replaces the interim `Unsupported` behaviour that the front-end changes specify for these constructs. **BREAKING** only for those interim requirements: queries that failed with `Unsupported` now return results.
- Add the typed error `PathLimitExceeded { limit }` for the search-state guard, and `Parse { dialect: Path, … }` for malformed path text.

## Capabilities

### New Capabilities
- `path-evaluation`: path expressions and their operators, the four v1 modes, bound-endpoint rule, hop caps and `max_hops`, cycle termination, memory guard, time-aware evaluation under every view, path result values and deterministic ordering.
- `layer-hops`: the virtual `sys:subject` / `sys:object` / `sys:predicate` hops and their inverses. They step from a statement to its parts and back, respect views, and appear in path values.
- `path-table-function`: the `tm_path` eponymous table-valued function (arguments, output columns, `path_json` format, composability, argument errors) and the `View::path` Rust API contract.
- `path-lowering`: how SPARQL property paths and Cypher variable-length, `shortestPath` and `allShortestPaths` patterns are evaluated, including endpoint binding, direction, time scoping, relationship isomorphism and unsupported path forms.

### Modified Capabilities
- None. `openspec/specs/` holds no archived capability yet. The interim requirements this change supersedes are in capabilities that are still unarchived (see Impact).

## Impact

- **Dependencies:**
  - `add-query-ir-and-sql-planner` (M1) is **required**. This change builds on its IR `PathPattern`, `View` and view-predicate function, its virtual predicates, its SQL codegen and region composition, and its table-valued-function extension point.
  - `add-sparql-frontend` (M2a) and `add-cypher-frontend` (M2b) are required for the `path-lowering` part only. The operator, `tm_path` and `View::path` can land before them.
- **Archive order:** archive `add-query-ir-and-sql-planner` first, then `add-sparql-frontend` and `add-cypher-frontend`, then this change. When this change is archived, delta specs added at archive time must **REMOVE or MODIFY** three interim requirements, pointing them to `path-lowering` and `path-table-function` so that no two capabilities contradict each other:
  - In `sparql-query`: the requirement "Property paths before the path engine". Paths other than a single IRI or `^iri` fail with `Unsupported { feature: "property path" }`.
  - In `cypher-read`: the interim requirement that variable-length relationships, `shortestPath` and `allShortestPaths` parse but return `Unsupported`.
  - In `sql-execution` (M1): the interim behaviour that a path pattern region fails with `Unsupported` until M3 implements the `tm_path` interface.

  The negated property set `!` stays `Unsupported` (see `path-lowering`).
- **Code:**
  - `tm-exec`: new `path` module (expression parser, NFA compiler, search modes, neighbour fetcher, result encoding), the `tm_path` virtual table's behaviour, and the planner routing of `PathPattern` regions.
  - `tm-rusqlite` (the first executor host): registration of `tm_path` and `rarray` on every connection, and the re-entrant connection handle used inside the virtual table.
  - `tm-ir`: `PathExpr` / `PathMode` completed, if M1 left them as placeholders.
  - `tm-sparql`, `tm-cypher`: lowering of path syntax.
  - `tiramemsu`: `View::path`, `PathRow`, `PathMode`, and the `OpenOptions.path_max_hops` default cap.
- **Crates:** paths need the executor host capability `vtab` (`lat.md/architecture#Executor`); `tm-exec` refuses to open on a host without it. In `tm-rusqlite`, this reuses the `rusqlite` `vtab` feature that M1 enables (for the eponymous virtual table), and adds the `array` feature (`rarray`, for batched neighbour fetches). No new external crates.
- **Errors:** `lat.md/api#Errors` gains `PathLimitExceeded`, and the `Parse` dialect gains `Path`.
- **Docs and tests:** `lat.md/query.md` (Path Engine) gains `@lat` code references. The specs `tests#Query#Paths Cross Layers` and `tests#Query#Unbounded Paths Are Capped` get implementations. Path benchmarks are added for `lat.md/roadmap#Benchmarks`.
