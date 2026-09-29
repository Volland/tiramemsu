## Why

After M0 (`add-core-store`) Tiramemsu can store, retract and time-travel statements, but it can only be read one pattern at a time through `View::triples`. SPARQL (M2a), Cypher (M2b) and the path engine (M3) all need one shared logical IR and one executor that turns that IR into correct, index-friendly SQL over the bitemporal `triple` table (`lat.md/query#Logical IR`, `lat.md/query#Physical Planning`). The IR has to exist before either front end, so that both dialects compile against the same algebra and the differential test suite has a single executor to compare (`lat.md/roadmap#Milestones`, decision D13).

## What Changes

- New crate `tm-ir`: the logical algebra from `lat.md/query#Logical IR` (TriplePattern, Join, LeftJoin, Filter, Union, Extend, Aggregate, Project, OrderLimit, PathPattern, Values), the expression and aggregate language, a `View` descriptor on every triple and path pattern, the query-level semantic flags (`match_mode`, `missing`, `graph_set`), eid binding, parameters, and structural validation.
- New crate `tm-exec`: the planner/router, constant encoding at plan time with short-circuiting of patterns that cannot match, the single view→SQL predicate function (`lat.md/query#Views and Scans`), virtual predicates (`lat.md/query#Views and Scans#Virtual Predicates`), SQL code generation per operator (`lat.md/query#Physical Planning#SQL Codegen`), execution on the right connection, and decoding of ObjectIds into typed values through an LRU term cache (`lat.md/storage#Term Dictionary`).
- Region routing: path patterns go to a native-operator interface exposed to SQL as the table-valued function `tm_path(start, path, mode, max_hops, view)`. This change defines only the interface and its SQL composition; `add-path-engine` (M3) implements it. Until then a path pattern fails with a typed `Unsupported` error. LFTJ is a routing extension point that stays disabled (M4).
- Facade (`tiramemsu`): `View::execute_ir(&IrQuery, &Params) -> Result<QueryResult>`, `View::explain_ir(…) -> Result<Explain>` for golden and plan tests, `QueryResult` / `ResultValue` types, `View::descriptor()` for front-end lowering, and planner/term-cache options on `OpenOptions`.
- One new error variant, `InvalidQuery { msg }`, for structurally invalid IR or missing parameters. It is added to the facade error enum next to `Unsupported`.
- No SPARQL or Cypher parsing, no path algorithm and no LFTJ operator are included.

## Capabilities

### New Capabilities

- `query-ir`: the logical operators, the per-pattern View, semantic flags (`match_mode`, `missing`, `graph_set`), eid binding, parameters, constants encoded at plan time, and short-circuiting of patterns that cannot match.
- `view-scoped-scans`: the single mapping from a View (Now / AsOf / History × Unfiltered / At) to SQL predicates, the verbatim `t_ret IS NULL`, covering-index use, per-pattern views mixed in one plan, and stable historical results.
- `virtual-predicates`: `sys:subject` / `sys:object` / `sys:predicate`, `tm:txAdded` / `tm:txRetracted` / `tm:validFrom` / `tm:validTo` / `tm:retractKind` resolved to row columns, and volatile keys visible only under Now.
- `sql-execution`: SQL codegen rules per operator, relationship isomorphism, set versus bag semantics, NULL versus unbound, ordering by decoded values, parameter binding, decoding results into typed values, choosing the reader or writer connection, and the extension point for native operators as table-valued functions.

### Modified Capabilities

None. `openspec/specs/` is empty. M0 (`add-core-store`) owns the store capabilities, and this change only consumes them.

## Impact

- **Depends on:** change `add-core-store` (M0). It uses `tm-core`'s ObjectId codec, term dictionary lookups, the `View` time selection and `TimeRef` resolution, the reader pool / writer connection, `Db::with`, and the `volatile` table. M0 must be merged first.
- **Unblocks:** `add-sparql-frontend` (M2a), `add-cypher-frontend` (M2b) and `add-path-engine` (M3), which lower to `tm-ir` and register native operators with `tm-exec`.
- **New crates:** `crates/tm-ir` (depends on `tm-core`) and `crates/tm-exec` (depends on `tm-ir` and `tm-core`, with `rusqlite` features `functions` and `vtab`). New dependencies: `lru` (term cache), `regex` (the `regexp` SQL function), `insta` (dev only, golden SQL snapshots).
- **Facade API:** additive only (`execute_ir`, `explain_ir`, `descriptor`, result types, options, one error variant). No breaking changes.
- **Database file:** no schema or format change. Every connection registers pure SQL helper functions at open time, and nothing is persisted.
- **Docs:** `lat.md/query.md` gets `@lat` code references, and `lat.md/api.md#Errors` gets the `InvalidQuery` row, in the final task.
