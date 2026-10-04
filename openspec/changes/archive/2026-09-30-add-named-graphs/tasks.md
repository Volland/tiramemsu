## Dependencies

- M0 (`add-core-store`), M1 (`add-query-ir-and-sql-planner`) and M2a (`add-sparql-frontend`) are archived and are the base of this change. It does not depend on `add-cypher-frontend`.
- Group 5 (paths) is a limit only. The path engine (`add-path-engine`) is not changed here.
- Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. The requirement text is in `specs/*/spec.md` of this change. Do not edit `lat.md/` until behaviour is implemented (see group 8).

## 1. Storage and core

- [x] 1.1 Add the constants `sys:inGraph` and `sys:Graph` to the reserved-namespace table in `tm-core`, and mark `sys:inGraph` as engine-owned in the reserved-namespace check. Unit-test that `INSERT`-style assert of `sys:inGraph` through `Tx::assert` fails with `ReservedNamespace` (spec `named-graphs` "Membership predicate is engine-owned").
- [x] 1.2 Run `EXPLAIN QUERY PLAN` on the two membership lookups: `(p = inGraph, o = g)` for `GRAPH <g>`, and `(s = eid, p = inGraph)` for a member's graphs. Confirm that the existing live indexes cover both without a table scan. If one is missing, stop and raise a format-version decision (design.md, Risks) before writing more code.
- [x] 1.3 Add `InvalidGraphName`, `GraphNotFound` and `GraphExists` to the facade error enum, with Display text that names the term or graph. Unit-test each variant's message.
- [x] 1.4 Implement `Tx::add_to_graph(eid, graph, opts)`: validate the graph id tag (`IRI`, `NODE`, `BNODE`), refuse `sys:` member statements, assert the membership idempotently through the existing assert path, and return `(membership_eid, new)`. Tests: idempotence, the two-graphs-one-eid scenario, literal and statement graph names, valid-time option.
- [x] 1.5 Implement `Tx::remove_from_graph`, `Tx::clear_graph` and `Tx::create_graph`, with the retract kind `explicit` for memberships and the declaration `(g, rdf:type, sys:Graph)` for `create_graph`. Tests: removing a non-member is a no-op, clear keeps member statements, create is idempotent.
- [x] 1.6 Implement `View::graphs()` and `View::graph_members(g)` on the view predicate function of M1, so time views apply. Tests: `asOf` before and after removal, `validAt` on a bounded membership, `History` listing removed memberships.
- [x] 1.7 Verify that the cascade retracts memberships of a retracted statement with `ret_kind` cascade, and that supersede and cardinality-one do not copy memberships to the replacement (spec "Membership is subject to predicate schema").

> Notes (group 1): the facade error enum is `tm_core::Error` (re-exported), so the three variants live there. `Tx::add_to_graph` takes `AssertOpts` (valid time via `opts.valid`), returns `(Eid, bool)`. Extra engine helpers for SPARQL: `Tx::drop_graph`, `graph_declared`, `graph_has_members`, `live_graphs`; `read::graphs` / `read::graph_members` back `View::graphs` / `View::graph_members`. Supersede skips `sys:inGraph` rows when replaying the cascade set (Tx::supersede would otherwise copy memberships). Task 1.2 result: both lookups are index seeks with no table scan and no format change. `(p=inGraph, o=g)` seeks `live_pos` (after churn plus ANALYZE) or, at equal cost on a store with no retracted memberships, `hist_pos`; the member row is joined by `INTEGER PRIMARY KEY`. `(s=eid, p=inGraph)` seeks `live_spo`. `sqlite_stat4` has samples for `live_pos` after ANALYZE.

## 2. IR and planner

- [x] 2.1 Add a graph selector to `TriplePattern` in `tm-ir` (`Any`, `Set(Vec<ObjectId>)`, `Var(VarId)`), in coordination with the M1 owners. Update the IR text form and its round-trip test. `Any` is the default so every existing plan is unchanged.
- [x] 2.2 Lower a non-`Any` selector to an extra join on the membership rows under the same view predicate, with `Var` binding the membership object. `Set` becomes an `IN` list over ids. Add snapshot tests of the generated SQL for `Set` of one, `Set` of several and `Var`.
- [x] 2.3 Make the planner statistics include `sys:inGraph` (skew on `o`). Add a plan test that `GRAPH <small>` over a store with a much larger graph seeks the small side first, and record the `sqlite_stat4` requirement per D19.
- [x] 2.4 Reject property paths, `shortestPath` and `tm_path` under a non-`Any` selector with `Unsupported { feature: "named graph path" }`, at planning time (spec "Unsupported combinations fail before execution").

> Notes (group 2): `GraphSel::Set` holds `TermOrVar` constants/parameters (not `ObjectId`s) so an unknown graph IRI short-circuits at plan time. The selector is lowered in `tm-exec` `plan/bind.rs` (`Binder::graph_join`), before normalisation: `Set([g])` and `Var` become a join with `(e sys:inGraph g)` under the pattern's own view, `Set` of several becomes `EXISTS { (e sys:inGraph ?x) FILTER ?x IN (..) }` so a statement in two listed graphs matches once. The internal eid variable is `~gsel<N>` and never a result column. Virtual predicates cannot carry a selector (validation error). Plan test: after `Db::optimize()` a `GRAPH <small>` pattern over a 1 800-member graph starts from the membership scan; without `ANALYZE` SQLite also needs `sqlite_stat4` to pick the small side (D19).

## 3. SPARQL front end

- [x] 3.1 Lower `GRAPH <g> { … }`, `GRAPH ?g { … }`, nested `GRAPH` and `VALUES`-bound graph variables to the selector. Remove the two `Unsupported("named graph")` and `Unsupported("GRAPH variable")` rejections. Keep the `tm:`-in-`GRAPH` `Parse` error that names `SERVICE`.
- [x] 3.2 Implement `FROM` and `FROM NAMED` with non-`tm:` IRIs, alone and together with `tm:` IRIs. Default graph with no `FROM` stays the union. Test every scenario in "Dataset clauses" and "Graphs combine with SERVICE time scopes".
- [x] 3.3 Make layer patterns (`~ ?r`, `{| … |}`) follow the surrounding `GRAPH` selection for the annotated statement, and read the annotation triples from the default view. Test the membership-layer scenario (`?e sys:inGraph <g1> ~ ?m {| v:addedBy ?who |}`).
- [x] 3.4 Confirm duplicate handling: a statement in two graphs appears once in the default graph, and once per membership under `GRAPH ?g`. Test with a predicate in `pred_multi` and with one that is not.

> Notes (group 3, and 2.4): the path limit is enforced in the SPARQL lowering (`GraphPattern::Path` while a graph selection is active, including a `FROM <g>` default graph), so it fails before any SQL; `PathPattern` carries no selector, so a `tm_path` call or Cypher `shortestPath` cannot be placed under a selector in the IR. Lowering keeps the active selection in `Lowerer` state (`dataset`, `active`); `GRAPH ?g` over a block with no triple pattern is `Unsupported("GRAPH ?g without a triple pattern")`. `FROM NAMED <g>` where `<g>` is left out of the list keeps the block's variables and adds `FILTER(false)`. Annotation triples whose subject is a reifier of the same BGP (`{| … |}`, `~ ?r`) and virtual-predicate patterns read with no graph selector; the annotated statement follows the surrounding `GRAPH`. A graph name that is a statement or transaction IRI fails with `InvalidGraphName`; a literal cannot be written as a graph name in SPARQL syntax (the parser rejects it), so the "Atomic failure" scenario uses a statement IRI.

## 4. SPARQL Update

- [x] 4.1 Implement `GRAPH` blocks in `INSERT DATA` and `INSERT` templates as assert plus `add_to_graph`, and `WITH <g>` as the graph of template triples outside their own `GRAPH` block. The report lists statements and memberships separately.
- [x] 4.2 Implement `GRAPH` blocks in `DELETE DATA` and `DELETE` templates as `remove_from_graph`, and keep plain deletes as retract with cascade. Test "Removing from one graph keeps the statement" and "Plain delete removes every membership".
- [x] 4.3 Implement `USING` and `USING NAMED` with graph IRIs for the `WHERE` pattern, and templates with `GRAPH ?g` when `WHERE` binds `?g`. Reject an unbound graph variable in a template with a `Parse` error.
- [x] 4.4 Implement `CREATE GRAPH`, `CLEAR GRAPH`, `CLEAR NAMED`, `DROP GRAPH`, `DROP NAMED` with `SILENT`, `GraphNotFound` and `GraphExists`. `LOAD`, `ADD`, `MOVE`, `COPY`, `CLEAR DEFAULT`, `CLEAR ALL`, `DROP DEFAULT` and `DROP ALL` keep failing with `Unsupported` naming the operation, before anything is written.
- [x] 4.5 Update the old scenarios that assert the removed rejections (`GRAPH in INSERT DATA`, `WITH`, `CLEAR`) to the new behaviour, and add the "Atomic failure" scenario.

> Notes (group 4): templates carry a graph (`GraphRef::{Default, Named, Var}`); `WITH` needs no code because spargebra rewrites it into `GRAPH <g>` on the templates plus `USING <g>`. `ADD`, `MOVE` and `COPY` are rewritten by spargebra into `DROP`/`INSERT` operations, so the request text is scanned (`parse::operation_keywords`) to name them. The report gains `TxReport::memberships` and `memberships_retracted`; `asserted` and `retracted` no longer list membership rows. Existing tests asserting the removed rejections were rewritten (sparql_update_tx, sparql_temporal, sparql_temporal_service, goldens `graph_*`, `update_with`, `update_create`, `update_clear`, `update_drop_silent`).

## 5. Interaction limits

- [x] 5.1 Return `Unsupported { feature: "USE GRAPH" }` for a Cypher `USE` that names a graph, and add a test.
- [x] 5.2 Add a TODO reference in `add-path-engine`'s design.md Open Questions that graph-filtered path search is the follow-up (a note, not a code change).

## 6. Facade, MCP and API docs

- [x] 6.1 Export the `Tx` and `View` methods from `tiramemsu`, with rustdoc examples that are doc-tested.
- [x] 6.2 Add an optional `graph` argument to the MCP write tool (adds membership) and to the MCP search tool (filters by graph). Add tool-schema tests. (Done with add-mcp-adapter: `assert` takes `graph`, `text_search` takes `graphs`; covered in `crates/tiramemsu-mcp/tests/server.rs`.)
  - **Deferred to the MCP milestone (M5):** the repo has no MCP crate yet, so there is no tool or schema to extend. The contract is documented in `lat.md/api.md`.

> Notes (group 6): `Tx` methods are the `tm_core::Tx` methods re-exported by the facade; `View::graphs` and `View::graph_members` are new, with a doctest. Task 6.2 is NOT done: the repository has no MCP server crate (`tiramemsu-mcp` is a later milestone, see `lat.md/api.md#MCP Tools`), so there is no write or search tool to extend and no tool schema to test. The `graph` argument is documented in `lat.md/api.md` for that crate.

## 7. Conformance and benchmarks

- [x] 7.1 Add the W3C SPARQL 1.1 dataset and update tests that this change turns from rejected to run to the allow-list harness. List every expected deviation with a reason: union default graph, and `CLEAR`/`DROP` keeping member statements. An unexpected pass or failure fails the build (D23).
- [x] 7.2 Add a benchmark on the 11M-statement fixture (`bench/engine-comparison/`): `GRAPH <g>` with a small and a large graph, `GRAPH ?g` over all graphs, and the storage ratio of a store where every statement has one membership. Record the numbers in design.md, Risks, before merging.
- [x] 7.3 Add a property test: for random adds, removes, retracts and clears, the set of `graph_members(g)` in the now view equals a model that tracks memberships as a set of `(eid, g)` pairs, and `asOf` any earlier transaction reproduces the model at that transaction.

> Notes (group 7): the W3C runner now loads `graphData` as named graphs (relative IRIs resolved against the test file directory, since no base IRI is configured) and runs graph queries and updates instead of skipping them; the historical substring filter still skips tests that only mention `WITHIN` and the like. Before: 615 passed, 136 failed (136 expected). After: 634 passed, 147 failed (147 expected): 19 more pass and 11 are listed (union default graph, membership-only delete, DROP keeps statements, GRAPH ?g over a subquery or a block with no triple pattern). Benchmark (11 M statements: storage ratio 1.92, `GRAPH <100 members>` 0.01 ms, 50 000 members 49 ms, 5 M members 550 ms, `GRAPH ?g` over all live statements 7.7 s; design.md Risks is not edited): `bench/named-graphs/bench.py`, results in `bench/named-graphs/results-<N>.json`, numbers in `lat.md/storage.md#Graph Memberships`. The property test is `crates/tm-core/tests/graph_props.rs`.

## 8. Documentation (when the behaviour above is implemented)

- [x] 8.1 Update `lat.md/data-model.md`: the Layers section's "There is no named-graph column" bullet (replace `sys:inContext` with `sys:inGraph`), and add a section on named graphs with links to this design.
- [x] 8.2 Update `lat.md/query.md`: the Temporal Syntax "v1 has no named graphs" bullet, the SPARQL front end section, and the Cypher `USE` note.
- [x] 8.3 Add a decision row (D28: graphs are tags, membership is a layer statement) to `lat.md/overview.md`, a `lat.md/tests.md` section for the new tests with `@lat` references, and run `lat check`.
- [x] 8.4 Archive this change with `openspec archive`, merging the delta specs into `openspec/specs/`.
