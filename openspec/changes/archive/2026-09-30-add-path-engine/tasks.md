Dependencies:
- `add-query-ir-and-sql-planner` (M1) must be merged before group 1: IR `PathPattern`, the view-predicate function, virtual predicates, SQL composition, the native-operator TVF extension point, and decoding.
- Groups 10–11 (lowering) also need `add-sparql-frontend` (M2a) and `add-cypher-frontend` (M2b). Groups 1–9 can land before them.

Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. The spec sections are in `specs/*/spec.md` of this change.

## 1. Scaffolding and prerequisites

- [x] 1.1 Create `crates/tm-exec/src/path/` with the submodules `syntax`, `ast`, `automaton`, `resolve`, `fetch`, `search`, `row` and `vtab` as empty stubs. Enable the `rusqlite` `array` feature in the host crate `tm-rusqlite` (not in `tm-exec`, which reaches SQLite only through the `Executor` trait). Verify that `cargo build -p tm-exec -p tm-rusqlite` passes.
  - Deviation: the `array` feature was already enabled by M1; the AST is `tm_ir::PathExpr` itself, so `ast` holds only the analyses (`nullable`). `PathMode` keeps M1's variant name `Reachability` (`PathMode::Reach` is an alias).
- [x] 1.2 Add `PathMode { Reach, Trail, AnyShortest, AllShortest }`, with parsing and display of `REACH|TRAIL|ANY_SHORTEST|ALL_SHORTEST` (case-insensitive), to `tm-ir`. Complete `PathExpr` if M1 left it as a placeholder. Add unit tests for round-tripping the names and for rejecting `WALK`, `SIMPLE`, `ACYCLIC` and `SHORTEST`.
- [x] 1.3 Add the error variants `PathLimitExceeded { limit }` and the `Path` parse dialect to the facade error enum. Add `OpenOptions.path_max_hops` (default 15) and `path_max_states` (default 1 000 000). Add a unit test for the defaults.
- [x] 1.4 Make the vocabulary/prefix resolver (`@vocab` bare names, CURIEs, reserved `sys:`/`tm:`) callable from `tm-exec`. If it lives only in `tm-cypher`, move it into `tm-core` without changing behaviour. Verify that existing tests still pass.
  - Deviation: the resolver moved to `tm_core::mapping::Vocab` (with `Vocab::load`); `tm-cypher` re-exports it and keeps `Name` resolution in the `VocabExt` trait.

## 2. Path text parser and AST

- [x] 2.1 Implement the AST: `Atom` (Pred, Unresolved, VirtualSubject, VirtualObject, VirtualPredicate, AnyRelationship), `Inverse`, `Seq`, `Alt`, `Star`, `Plus`, `Opt`, `Repeat{min,max}`, plus a `nullable()` analysis. Add unit tests for `nullable` on every operator.
- [x] 2.2 Implement the recursive-descent path text parser. It covers the SPARQL 1.1 precedence (`|` < `/` < `^` < postfix), parentheses, `{m,n}`/`{m,}`/`{n}`, `<iri>`, CURIEs, bare names, `sys:anyRelationship` and whitespace. Add table-driven tests for every operator and for the precedence cases in spec `path-evaluation` "Path expression operators".
- [x] 2.3 Add parse errors with byte-offset spans (`knows//likes`, `(knows`, `{3,1}`) and unknown-prefix errors. Test them against spec `path-table-function` "Path expression text" (Malformed path text, Unknown prefix).
- [x] 2.4 Implement atom resolution. It maps IRIs to ObjectIds through the dictionary and turns missing IRIs into `Unresolved`, which never matches. It maps `sys:subject`/`sys:object`/`sys:predicate` to virtual atoms and `sys:anyRelationship` to the wildcard. Test the "Unknown predicate matches nothing" scenario.
- [x] 2.5 Implement the IR `PathExpr` → AST conversion, so the planner avoids reparsing, and the canonical text printer (full IRIs). Add a property test that print → parse → AST is the identity on random ASTs.
  - Deviation: no IR-to-AST conversion is needed (same type). The canonical printer is `tm_ir::display::path_text_canonical`, which never uses the `v:` prefix (it follows `@vocab` when parsed).

## 3. Automaton

- [x] 3.1 Implement the Thompson NFA over `(Atom, Direction)` symbols, including `Inverse`, which pushes direction down, and the bounded-repetition unrolling of design Decision 3. Add unit tests for the state counts of `p{2,3}`, `p{2,}` and `(p/q)+`.
- [x] 3.2 Implement ε-closure and subset-construction determinisation, with alphabet refinement when `AnyRelationship` and concrete predicates co-occur. Add a 4 096-state guard that raises `Unsupported { feature: "path expression too complex" }`. Test that `p|p` and `(p*)*` yield a DFA with a single run per word.
- [x] 3.3 Add a property test: for random small expressions and random words, NFA acceptance equals DFA acceptance.

## 4. Neighbour fetcher

- [x] 4.1 Implement `NeighbourFetcher` for stored predicates, out and in. It uses the executor's cached statements (`prepare_cached` in `tm-rusqlite`), with SQL keyed by (kind, direction, view shape), `rarray(?1)` chunks and bind parameters only, with view predicates taken from M1's view-predicate function. Unit-test that the `Now` SQL contains the verbatim `t_ret IS NULL`.
  - Deviation: the host binds the array as a new `SqlValue::IntArray`, and the fetch SQL is `FROM rarray(?1) AS r CROSS JOIN triple AS t`, because `IN rarray(?1)` made SQLite scan `triple`.
- [x] 4.2 Implement the virtual-hop fetch shapes (`sys:subject`/`sys:object`/`sys:predicate`, forward by rowid on `STMT` ids only, inverse by `s`/`o`/`p IN`). Test each against spec `layer-hops` "Forward virtual hops" and "Inverse virtual hops".
  - Deviation: the virtual predicate in a path value is a reserved id outside the dictionary (`tm_exec::path::row::virtual_pred_id`), since a reader cannot intern `sys:subject`.
- [x] 4.3 Implement the `AnyRelationship` fetch and the relationship-view filter (`isEdge` true/false sets, `rdf:type`, `sys:` IRIs, non-literal object tags). Test spec `path-evaluation` "Wildcard skips properties and labels", "Wildcard honours isEdge" and the `sys:anyRelationship` text scenario.
- [x] 4.4 Implement `HopKey` ordering (eid, kind Stored<Subject<Object<Predicate, dir Out<In), grouping by `from`, and merging back in frontier order. Add a property test that batch sizes 1, 7 and 256 give identical neighbour sequences.
- [x] 4.5 Add `EXPLAIN QUERY PLAN` tests for every fetch shape under Now, AsOf, History and ValidAt, asserting the covering `live_*`/`hist_*` index or rowid lookup from the design Decision 5 table (extends the cases of `tests#Storage Invariants#Views Use Covering Indexes`, which another change owns; do not add a second `@lat` ref).

## 5. Search modes

- [x] 5.1 Implement `StateBudget` and the shared layer driver (initial `(start, q0)`, zero-length emission for nullable expressions, depth bound, per-layer grouping by symbol). Test the zero-length scenarios for an isolated IRI and a literal start.
- [x] 5.2 Implement `ReachSearch` (visited `(node,state)`, emitted ends, per-layer sort by end). Test all "Reachability mode semantics", "Termination on cycles" (reach and self-loop) and "Reachability order" scenarios.
- [x] 5.3 Implement `TrailSearch` (arena with parent pointers, hop-identity check along the parent chain, arena-order emission). Test "Trail mode semantics", "Trail on a cycle" and "Trail order", including parallel edges and "A relationship is not reused".
- [x] 5.4 Implement `ShortestSearch` for `ANY_SHORTEST` (first predecessor, lexicographically smallest tie-break). Test "Any shortest picks the minimal length", "Any shortest is deterministic among ties" and "Shortest on a cycle".
- [x] 5.5 Implement `ALL_SHORTEST` (predecessor lists, per-layer target marking, forward DFS in HopKey order over the marked DAG). Test "All shortest returns every minimal path" and "All shortest does not duplicate ambiguous matches".
- [x] 5.6 Implement end-filter early termination for every mode, plus reversed evaluation (inverse expression, then reverse nodes and edges and flip dir at row build). Test "Shortest with both endpoints fixed" and "Only the end is bound".
  - Deviation: reversed evaluation lives in the planner and decoder (`Dom::PathJson { reversed }`), not in the engine.
- [x] 5.7 Implement the `max_hops` semantics and the memory guard. Test "Hop limits" (max_hops bounds reachability) and "Guard trips on explosive trails" (limit 1 000, complete graph K20, `PathLimitExceeded { limit: 1000 }`, no partial rows).
- [x] 5.8 Add a property test (proptest): on random small graphs, REACH ends equal a naive fixpoint over the product graph; TRAIL paths equal brute-force enumeration of edge-distinct paths up to `max_hops`; ALL_SHORTEST paths equal all brute-force paths of minimal length per end.

## 6. Time awareness and layer hops

- [x] 6.1 Test every "Time-aware evaluation" scenario (Now vs AsOf, retract, History, ValidAt, supersede) through the engine API.
- [x] 6.2 Test "Historical path results are stable": compute AsOf(t) paths, then run retract, supersede and cascade operations, then recompute and compare (extends `tests#Time Travel#Historical Reads Are Stable` to paths; no second `@lat` ref).
- [x] 6.3 Test the `layer-hops` "Paths cross layers" scenarios (belief → statement → entities, entity → beliefs, arbitrary depth, mixed recursive walk) (`// @lat: [[tests#Query#Paths Cross Layers]]`).
- [x] 6.4 Test the `layer-hops` "Virtual hops respect the view" and "Virtual hops in path values and trails" scenarios, including virtual-hop trail identity (eid, kind).
- [x] 6.5 Test "Asserting a virtual hop predicate is rejected" (`ReservedNamespace`), together with the path from a non-statement returning nothing.

## 7. Path rows and path_json

- [x] 7.1 Implement `PathRow`, `Path` and `Hop`, and the `path_json` serialiser (`{"nodes":[…],"edges":[{"eid","p","dir"}]}`, integers). Test the "Path value contents", "Reachability rows carry no path" and "Zero-length row" scenarios.
- [x] 7.2 Add a JSON round-trip test: `path_json` parses in SQLite `json_each`, and edge `eid` values equal `triple.eid`.

## 8. tm_path virtual table

- [x] 8.1 Implement `TmPathVTab` as an eponymous-only read-only module with the declared schema (visible `start, "end", hops, path_json`, and HIDDEN `arg_start, path, mode, max_hops, view`). Register it and `rarray` on every reader and writer connection at open through the executor host: in `tm-rusqlite`, with `eponymous_only_module` and the `array` feature's `rarray`. Make `tm-exec` refuse to open, with an error naming the capability, on a host that does not declare `vtab`. Test "Callable on a reader connection", "Writes are rejected", "Select star columns", and the refusal with a stub host lacking `vtab`.
  - Deviation: the host API is `HostRegistry::register_conn_table` (`tm_core::ConnTableFunction`); rows are materialised per call, so `LIMIT` only stops the search through the pushed-down `"end"`.
- [x] 8.2 Implement `best_index`: required EQ on `arg_start` and `path` (otherwise `SQLITE_CONSTRAINT`), optional `mode`/`max_hops`/`view`/`"end"` encoded in `idxNum`, and costs. Test "Correlated start from another table" and "End constraint".
- [x] 8.3 Implement `filter` argument validation and the `tm_path: <arg>` error messages, the NULL-start empty cursor, mode and `max_hops` defaults, and view text parsing (short and full `tm:` IRI forms, `asOf/<instant>` resolution). Test every "tm_path argument errors" and "View argument text" scenario.
- [x] 8.4 Implement the re-entrant non-owning connection wrapper (in `tm-rusqlite`), the per-connection compiled-path LRU and the typed-error slot. Test "Callable inside speculation", "Concurrent commit is not seen mid-query", and that a `PathLimitExceeded` inside SQL surfaces as the typed error through the executor.
  - Deviation: the non-owning connection is created once per connection in `tm-rusqlite` and its statement cache is flushed before the owner closes; the compiled-path LRU sits in `PathEngine`, shared by all connections.
- [x] 8.5 Test SQL composability: "Aggregate over paths", "JSON inspection of hops", "Default trail cap", "Explicit max_hops", "Mode is case-insensitive" and "Minimal call".

## 9. View::path API

- [x] 9.1 Implement `View::path(start, path, mode, max_hops)` on the facade, on a reader or on the writer inside `with()`, returning `Vec<PathRow>` in deterministic order. Test "API matches tm_path", "API uses the view's valid time" and "API returns path values".
- [x] 9.2 Test "API parse error" (`Parse { dialect: Path, span }`) and "API inside speculation" (visible inside, absent after).

## 10. Planner integration (M1 extension point)

- [x] 10.1 Implement the `PathPattern` region lowering in the planner: anchor choice (start, else end with inverse and `reversed`, else `Unsupported`), canonical path/view text parameters, and the `max_hops` policy (SPARQL NULL, explicit Cypher bound, or `path_max_hops`). Add golden `explain_ir` snapshots for start-bound, end-bound, both-bound and same-variable cases.
- [x] 10.2 Implement far-end binding, `"end" = ?` pushdown and `"end" = start` for the same variable at both ends, plus `bind_path` → `path_json`/`hops` columns and path decoding with reversal. Test "Only the end is bound" through IR.
- [x] 10.3 Implement the nullable-expression fallback for a constant start missing from the dictionary (zero-length binding to the constant term). Test SPARQL "Zero-or-more from a term not in the graph".
- [x] 10.4 Remove M1's interim `Unsupported` for `PathPattern` regions. Verify that M1's routing tests now route paths to `tm_path`.
  - Deviation: `route::precheck` keeps `Unsupported` for a bare `QueryEngine` with no registered path operator; `Db` always registers `tm_path`.

## 11. Front-end lowering

- [x] 11.1 SPARQL: lower `*`, `+` and `?` paths (spargebra → `PathExpr`, REACH, view from `FROM`/`SERVICE`). Replace the interim "Property paths before the path engine" behaviour. Test every scenario in spec `path-lowering` "SPARQL recursive property paths".
- [x] 11.2 SPARQL: expand non-recursive paths (`/`, `|`, `^`) with the SPARQL 1.1 translation, reject negated property sets, and enforce the bound-endpoint rule. Test "SPARQL non-recursive property paths", "SPARQL endpoint binding" and "Unsupported SPARQL path forms".
- [x] 11.3 SPARQL: test "SPARQL paths honour temporal scope" and "SPARQL paths across layers" (a path variant of `tests#Query#Per Pattern Time Scopes`; no second `@lat` ref).
- [x] 11.4 Cypher: lower variable-length relationships (all quantifier forms, type alternation, wildcard, `->`/`<-`/`-`, TRAIL, default cap). Test every scenario in "Cypher variable-length relationships", including "Unbounded pattern stops at the cap" (`// @lat: [[tests#Query#Unbounded Paths Are Capped]]`).
- [x] 11.5 Cypher: build path values (`length`, `nodes`, `relationships`, relationship-list variables, synthetic virtual-hop relationships). Test "Cypher path and relationship-list bindings".
- [x] 11.6 Cypher: add the relationship-isomorphism filters between path regions and fixed relationships, and between two path regions. Test "Cypher relationship isomorphism with paths".
  - Deviation: relationship isomorphism between path regions and fixed relationships is checked on the result rows in `tm-cypher` (`absorb`), not with `json_each` filters in SQL.
- [x] 11.7 Cypher: lower `shortestPath` and `allShortestPaths` (min ∈ {0, 1} check, cap, `OPTIONAL MATCH` NULL). Test "Cypher shortest paths".
- [x] 11.8 Cypher: add the bound-endpoint rule, `USE`/`CALL { USE … }` scoping and the unsupported forms (property maps, QPP, `REPEATABLE ELEMENTS` with var-length). Test "Cypher endpoint binding", "Cypher paths honour temporal scope" and "Unsupported Cypher path forms".
- [x] 11.9 Replace the interim `cypher-read` `Unsupported` behaviour for these constructs. Verify that the M2b tests expecting `Unsupported` are updated or removed.

## 12. Differential tests

- [x] 12.1 Add SPARQL/Cypher path pairs to the differential corpus (cycles, parallel edges, diameter < 15, asOf and now). Assert that SPARQL `+` ends equal Cypher `DISTINCT` trail ends (path cases added to the corpus behind `tests#Query#Differential SPARQL Cypher`; no second `@lat` ref). This covers spec `path-lowering` "Dialect agreement on paths".
- [x] 12.2 Add a cross-surface test: for random fixtures, `View::path`, `tm_path` and the equivalent Cypher/SPARQL query give the same endpoint sets and hop counts.

## 13. Benchmarks

- [x] 13.1 Add a `criterion` bench `crates/tm-exec/benches/path.rs` with synthetic power-law graphs of 10⁶ statements (10⁷ behind an env flag). Measure ANY_SHORTEST latency between random bound pairs under Now and AsOf (`lat.md/roadmap#Benchmarks` "Paths").
  - Deviation: the benches live in `crates/tiramemsu/benches/path.rs` (beside `core.rs` and `sparql.rs`), not in `tm-exec`.
- [x] 13.2 Add a 3-hop TRAIL latency bench (`p{1,3}` and `sys:anyRelationship{1,3}` from random starts) under Now, AsOf and ValidAt, and record rows per second.
- [x] 13.3 Add a REACH bench (`p+` over a 10⁶ chain and over a small-world graph) and an ALL_SHORTEST bench on a grid. Record results and the `rarray` chunk sizes tried (64, 256, 1024) in the bench README, and set the default chunk size from them.

## 14. Documentation and wrap-up

- [x] 14.1 Update `lat.md/api.md` Errors with the `PathLimitExceeded` row and the `Path` parse dialect. Document `OpenOptions.path_max_hops` and `path_max_states`, and the `tm_path` argument, view and `path_json` formats, in `lat.md/query.md` Path Engine.
- [x] 14.2 Before archiving, confirm that M1, M2a and M2b are archived. Add the archive-time delta specs that REMOVE/MODIFY `sparql-query` "Property paths before the path engine", the interim path requirement in `cypher-read`, and the interim path behaviour in `sql-execution`. Run `openspec validate --strict`.
- [x] 14.3 Update lat.md (query.md) with `@lat` code refs (path module, searchers, fetcher, vtab, planner region), and run `lat check` until it passes.
