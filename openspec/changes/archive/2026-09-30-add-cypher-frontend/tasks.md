Depends on M0 `add-core-store` and M1 `add-query-ir-and-sql-planner`, which must be merged first. Runs in parallel with M2a `add-sparql-frontend`. Group 13 (the differential suite) needs M2a to finish before its pairs can pass. M3 `add-path-engine` will later modify `cypher-read` to evaluate path patterns. Test code references lat.md specs with `// @lat: [[tests#…]]`, one comment per spec, placed next to the covering test.

## 1. Crate scaffold and parser

- [x] 1.1 Create `crates/tm-cypher` (deps: `open-cypher = "=0.2.1"`, `tm-ir`, `tm-core`). Add it to the workspace. Stub `compile()`, `CypherProgram` and `CypherError`. Verify with `cargo build -p tm-cypher`.
- [x] 1.2 Parser spike: parse every query in the `cypher-*` spec scenarios with `open_cypher::parse` after blanking extensions by hand. Record gaps in `crates/tm-cypher/PARSER.md`. Apply the fallback trigger from design Decision 1 (first fallback: vendor oxilite's hand-written lexer, parser and AST; then `decypher`). Done when every in-scope construct parses, or when the switch decision is recorded.
- [x] 1.3 Define the internal `ast.rs` subset (clauses, patterns, expressions, spans) and a `Span` type holding byte offsets into the original text.
- [x] 1.4 Implement `parse/adapter.rs` for read clauses: `MATCH`, `OPTIONAL MATCH`, `WHERE`, `WITH`, `RETURN`, `ORDER BY`/`SKIP`/`LIMIT`, `UNWIND`, `UNION`, `CALL {}`, `EXISTS {}`, `CALL proc()`. Unit tests assert AST shape and spans.
- [x] 1.5 Adapter for expressions: literals, parameters, operators, `CASE`, list and map literals, map projection, list comprehension, function calls, indexing and slicing.
- [x] 1.6 Adapter for write clauses: `CREATE`, `MERGE` (with `ON CREATE`/`ON MATCH`), `SET` (all forms), `REMOVE`, `DELETE`, `DETACH DELETE`.
- [x] 1.7 Adapter maps variable-length relationships, `shortestPath`, `allShortestPaths`, `FOREACH`, `LOAD CSV`, quantified path patterns, GQL path modes, pattern comprehension and label expressions `! & %` to `ast::Unsupported(feature, span)`. The test runs each spec scenario and expects `Unsupported`, not `Parse`.

## 2. Extension pre-pass

- [x] 2.1 Implement `parse/prepass.rs`: detect scope starts over `open_cypher::lex` tokens (query start, after `UNION [ALL]`, after `CALL {`, after the importing `WITH` of a CALL body).
- [x] 2.2 Parse time selectors `USE (AS OF e | HISTORY)? (VALID AT e)?`, where `e` is an integer literal, a parameter, `datetime('…')` or `date('…')`. Record them in `ScopeExt` and blank them with spaces of the same byte length. Report `Parse` for two tx selectors or a float literal, and `Unsupported` for `USE <graphname>`.
- [x] 2.3 Detect `REPEATABLE ELEMENTS` and `DIFFERENT RELATIONSHIPS` after `MATCH`/`OPTIONAL MATCH`, and record them in `MatchExt`. Leave a misplaced `USE` in place so the upstream parser reports it.
- [x] 2.4 Property test (`proptest`): for random queries with injected extensions, every upstream span maps to the same bytes of the original text. Include the scenario "Span refers to original text after extensions". *Note: open-cypher 0.2.1 is kept (PARSER.md). It has no CALL {} clause, FOREACH or LOAD CSV, so parse/subq.rs cuts CALL subqueries out and parses them recursively; FOREACH/LOAD CSV/schema commands are recognised on parse failure.*

## 3. IR readiness and semantic analysis

- [x] 3.1 Verify that `tm-ir`/`tm-exec` provide `Lookup` (a correlated scalar with list-if-many), `Exists`/`NotExists`, null-safe join, `RowNumber` and `Unnest`. They are specified in add-query-ir-and-sql-planner (query-ir requirements and task group 12). Done when each has an IR constructor and a codegen test in M1.
- [x] 3.2 Implement `sema/scope.rs`: variable kinds `Node`/`Rel`/`Value`/`Path`, the `dual_used` flag, `WITH`/`RETURN` scope closing, CALL imports, and the shadowing check. Tests cover the out-of-scope, undefined-variable and shadowing scenarios.
- [x] 3.3 Implement `sema/check.rs`: aggregate placement, `UNION` columns, missing parameters, `SKIP`/`LIMIT` literals, a write clause on a read-only view, a write combined with a non-Now top-level `USE`, and a node variable used in relationship position. Each check returns a `Parse` or `Unsupported` error with the spec's span. *Note: IR primitives verified in tm-exec/tests/cypher_primitives.rs (Lookup, null-safe join, RowNumber, Unnest, Exists). Design deviation: tm-cypher uses IR for graph patterns and lookups only; expression, projection and aggregate evaluation runs in a Rust interpreter (see task 6.x notes).*
- [x] 3.4 Map `CypherError` to the facade `Error::{Parse{dialect: Cypher, span, msg}, Unsupported{feature}, Eval{dialect, msg}}`. Add the `DeleteConnectedNode` and `Eval` variants to the facade.

## 4. Vocabulary mapping

- [x] 4.1 Implement `vocab.rs`: `resolve()` and `render()` per design Decision 9, with built-in prefixes, percent-encoding, longest-prefix tie-break, and the local-name-with-colon fallback. Unit tests cover every `vocabulary-mapping` rendering scenario.
- [x] 4.2 Build `CompileCtx`: read the current `sys:vocab` and `sys:prefix` layer, and the schema flags (`isEdge`, `unique`, `cardinality`) per distinct view, on the same connection and snapshot that execution will use.
- [x] 4.3 Resolve `@id` values: CURIE, absolute IRI, and the skolem IRIs `urn:tiramemsu:{node,bnode,stmt}:<n>` → ObjectId. Add the `elementId`/`id` functions. Test the round trip for every kind.
- [x] 4.4 Facade helpers `Tx::set_vocab(iri)` and `Tx::set_prefix(name, iri)`: replace the live setting, and reject built-in prefix names with `ReservedNamespace`. Tests: "Change @vocab", "Reserved prefix name" and "Historical query uses current vocabulary".
- [x] 4.5 Implement the reserved implicit labels `Statement` and `Predicate`, and `sys:` hiding in labels, keys, properties, untyped patterns, node scans and procedures. Tests cover the `vocabulary-mapping` sys-hiding scenarios.

## 5. Read lowering: patterns

- [x] 5.1 Node patterns: label generator with `Project{distinct}`, extra labels as `Exists`, `:A|B` as `Union`, the node scan for bare `(n)` with the C4 exclusions. Golden IR snapshot tests.
- [x] 5.2 Property-map constraints as existence tests, with Cypher numeric equality (`{score: 30.0}` matches `INT 30`). Test "Duplicate label statements do not duplicate rows".
- [x] 5.3 Relationship patterns: direction, undirected as a `Union`, `[:A|B]`, untyped with `notSys`, `≠ rdf:type`, and `relClass` using the static schema snapshot. Test "Parallel edges are distinct rows".
- [x] 5.4 `sys:isEdge` true/false overrides and `rdf:type` exclusion (C1, C7). Tests cover all `cypher-read` statement-classification scenarios.
- [x] 5.5 Relationship isomorphism: pairwise `eid ≠` filters per `MATCH` clause, skipped under `REPEATABLE ELEMENTS`, with `DIFFERENT RELATIONSHIPS` accepted. Tests cover all isomorphism and opt-out scenarios.

## 6. Read lowering: expressions and clauses

- [x] 6.1 Property access `x.k` → `Lookup` with list-if-many, distinct values in eid order, and `null` propagation. `keys()`/`properties()` lookups. Test "Multi-valued property becomes a list".
- [x] 6.2 Volatile fallback in `Lookup` under tx `Now` only, with the triple winning. Include volatile keys in `keys()`/`properties()` under `Now`. Tests cover all volatile scenarios in `cypher-read`.
- [x] 6.3 Three-valued logic for `AND`/`OR`/`NOT`/`XOR`, comparisons, `IN`, `IS [NOT] NULL`, `STARTS WITH`/`ENDS WITH`/`CONTAINS`, and `=~` (register a SQLite regex function). Truth-table unit tests.
- [x] 6.4 Built-in function registry `funcs.rs` (the list in `cypher-read`), `CASE`, map projection, list comprehension over lists, indexing and slicing. Unknown function → `Unsupported`.
- [x] 6.5 `tm_raise` runtime error function and `Eval` mapping: type errors, integer division or modulo by zero. Lenient conversions return null.
- [x] 6.6 `OPTIONAL MATCH` → `LeftJoin` with `WHERE` as the join condition. `WITH`/`RETURN` projection, aliases, expression-text column names, `RETURN *` sorted, `DISTINCT` with null-equivalence.
- [x] 6.7 Aggregation: implicit grouping, `DISTINCT` aggregates, null skipping, and the empty-input single row (`count` 0, `collect` []).
- [x] 6.8 `ORDER BY` with Cypher global ordering (decoded values, nulls last or first), and `SKIP`/`LIMIT` literals and parameters with negative checks.
- [x] 6.9 `UNWIND` (constant `Values`, computed lists via `Unnest`, null/empty → 0 rows, scalar → 1 row), and `UNION`/`UNION ALL`.
- [x] 6.10 `EXISTS {}` and pattern predicates → `Exists`/`NotExists`. Test "Pattern predicate does not multiply rows".
- [x] 6.11 Uncorrelated `CALL {}` as a cross join. Correlated `CALL { WITH … }` via decorrelation, null-safe join, per-row aggregate defaults and window-based per-row `LIMIT`. Tests cover all CALL scenarios.
- [x] 6.12 Fixed-length named paths: `Path` value, `nodes()`, `relationships()`, `length()`. Built-in procedures `db.labels`, `db.relationshipTypes`, `db.propertyKeys`, and unknown procedure → `Unsupported`.
- [x] 6.13 Parameters: binding, missing-parameter `Parse` error, and parameters in property maps, `SKIP`/`LIMIT`, `UNWIND` and `USE`.

## 7. Values and results

- [x] 7.1 `value.rs`: the `CypherValue` enum, `NodeValue`/`RelValue`/`PathValue`, Cypher equality and ordering. Unit tests for the cross-type order.
- [x] 7.2 Decode ObjectIds to Cypher values per the tag table in design Decision 10, including DATETIME to DateTime with its stored offset (`tz ≠ 0`) or LocalDateTime (`tz = 0`), dropped language tags, typed literals as lexical strings, and IRI or skolem strings for `isEdge false`. Tests: "Scalar round trip of types", "DateTime equality compares instants" and "Date-time without timezone reads as LocalDateTime". These cover the Cypher `=` half of `tests#ObjectId#DateTime Keeps Its Offset`; that spec keeps exactly one `@lat` reference, on the codec-level test outside this change, so none is added here.
- [x] 7.3 Build Node and Relationship values: element ids, rendered labels (with `Statement` first for statements), properties without `sys:` or temporal names. Test "Node and relationship values".
- [x] 7.4 JSON encoding of `CypherResult` (design Decision 10), and the facade `QueryResult::Cypher` variant. Snapshot tests. *Note: the facade has no QueryResult enum (tm_exec::QueryResult is the IR result struct and M2a returns SparqlResult), so View::cypher returns tm_cypher::CypherResult directly (re-exported as tiramemsu::CypherResult) with report: Option<TxReport>; the JSON encoding is CypherResult::to_json with an insta snapshot in tests/values.rs.*
- [x] 7.5 `View::cypher(q, params)` in the facade: compile with the view's default, run on a reader connection, decode. Tests: "Read query on the current view", "Write clause on a read-only view is rejected" and "View time selection is the default".

## 8. Dual view

- [x] 8.1 A relationship eid variable as a node-position term. The isomorphism filter ignores node-position uses. `// @lat: [[tests#Query#Dual View Binds Same Eid]]` goes on the Cypher-side unit test of the lat.md example query.
- [x] 8.2 `:Statement` filter and generator. Node-position variables bound to `STMT` decode as Node `:Statement`. Tests: "Enumerate statements", "Typed statement" and "Statement nodes are not plain nodes".
- [x] 8.3 `startNode`/`endNode`/`type` via `sys:subject`/`sys:object`/`sys:predicate` on either form, with the literal end for property statements.
- [x] 8.4 Properties and relationships of statements (two-level layers), returned form by first binding, and `r = x` equality across forms. Tests cover the remaining `cypher-dual-view` read scenarios.

## 9. Temporal clauses

- [x] 9.1 `lower/time.rs`: a scope stack of `TimeSel` merged selector by selector (handle → query → CALL body → nested). `AS OF` instant → `t` resolution at compile time. The empty view for `t < 1` or instants before the first tx.
- [x] 9.2 Apply `σ` to every `TriplePattern`, `Lookup`, label test and virtual predicate in scope. Tests: "Superseded value as of an earlier tx", "Half-open interval" and "All versions of a relationship".
- [x] 9.3 Per-pattern `CALL { USE … }`, with imported variables whose lookups inside the body use the body's view. `// @lat: [[tests#Query#Per Pattern Time Scopes]]` on "What changed since tx 150".
- [x] 9.4 Statement time properties `txAdded`/`txRetracted`/`validFrom`/`validTo` and the `tm:` CURIEs, including `tm:retractKind`. Temporal names win on statements and stay ordinary keys on nodes, and they are excluded from `keys()`. Tests cover all "Statement time properties" scenarios.
- [x] 9.5 Argument validation: integer or parameter for `AS OF t`, date or datetime for `VALID AT`, placement errors. Tests cover the placement and invalid-argument scenarios.

## 10. Write program and CREATE/MERGE

- [x] 10.1 `program.rs`: `CypherProgram` with `Read`/`Write`/`Merge`/`DeleteCheck` steps, eager boundaries after writes, seed rows passed as `Values`, and write expressions pre-computed as `Extend` columns.
- [x] 10.2 Facade executor: `Tx::cypher` (runs inside `transact`) and `Db::cypher_write` (one transaction, rows + `TxReport`). A failure rolls back with no trace. Tests: "Create and return in one transaction", "Later clauses see earlier writes", "Failure leaves no trace" and "Transaction handle composes with API operations".
- [x] 10.3 Write value encoding (the Integer range to `INT`/`TYPED`, a list as several statements, map/null-in-list → `Eval`, DateTime → instant plus offset, a named zone → its resolved offset with the zone name dropped, LocalDateTime → no timezone; never normalised to UTC). Property-map constants of type DateTime compare by instant. Tests cover the value-encoding scenarios, including "DateTime keeps its offset", "LocalDateTime is a date-time without timezone", "Named zone stored as its offset" and "Same instant with another offset is a different value".
- [x] 10.4 `CREATE` nodes: `new_node`, `@id` IRI identity, labels and properties via assert, and an empty node writing nothing (C11).
- [x] 10.5 `CREATE` relationships via `create` (parallel edges), relationship property statements on the eid, `validFrom`/`validTo` map keys as the valid interval, path creation, and dual-view endpoints. Test "Parallel edges" (the M0 test already owns `tests#Operations#Create Makes Parallel Edges`, so no `@lat` reference is added here).
- [x] 10.6 `MERGE` with a `sys:unique` key → `upsert` plus idempotent asserts, first unique key in map order, `ON CREATE`/`ON MATCH`.
- [x] 10.7 `MERGE` by pattern match on the writer connection (node and relationship patterns, bind all matches, create the whole pattern otherwise, null → `Eval`). A concurrency test with two threads runs `MERGE` on the same key.

## 11. SET, REMOVE, DELETE

- [x] 11.1 `SET x.k = v` with the seven-rule decision list (C8) over live property statements. One test per rule, including supersede with replayed annotations and cardinality-one without replay.
- [x] 11.2 `SET x += m`, `SET x = m`, `SET n:L`, `REMOVE n:L`, `REMOVE x.k` (with cascade), and `SET x = entity` → `Unsupported`.
- [x] 11.3 `SET r.validFrom/validTo` → `supersede` and variable rebinding. `SET r.txAdded` → `Unsupported`. An inverted interval → `InvalidPatch`.
- [x] 11.4 `DELETE` of a relationship or statement (retract with cascade, no-op for retracted or null), and `DELETE` of a node (properties and labels) with the end-of-query `DeleteCheck` → `DeleteConnectedNode`. Tests cover all DELETE scenarios.
- [x] 11.5 `DETACH DELETE` retracts every live statement with `s = n` or `o = n`, and history keeps the node visible. Schema errors propagate (`ValueTypeMismatch`, `UniqueViolation`, `ReservedNamespace`, `CascadeLimitExceeded`) with no trace.
- [x] 11.6 Write-with-time rules: a top-level non-Now `USE` → `Unsupported`, and a `CALL { USE AS OF … }` read feeding a `SET` (test "Restore a past value from a historical scope"). Dual-view writes: tests "Attach a belief to a relationship", "Annotate a relationship" and "Delete a statement through its node form". *Note: Sections 5-11 share one deviation from the design text: tm-cypher lowers graph patterns, label and property existence tests, isomorphism and time views to the IR (golden snapshots in crates/tm-cypher/tests/golden_ir.rs), while expressions, functions, projection, aggregation, ordering, UNWIND, UNION, CALL and the write clauses are interpreted in Rust over rows of Cypher values (crates/tm-cypher/src/exec/*). Reason: the M1 IR has only SPARQL scalar functions and no list/map values, so SQL lowering would need about 80 new IR functions. Consequences: no tm_raise (runtime errors are Eval errors raised by the interpreter), no Read/Write/Merge/DeleteCheck step list (the interpreter runs the AST clause by clause; program.rs holds the checked AST), the schema flags are read through IR per view at run time instead of a CompileCtx snapshot, and volatile values are read only under {Now, Unfiltered} (the IR restriction). The tm-core Tx gained set_extension/extension (so Tx::cypher can reach the query engine), set_vocab and set_prefix. The existing @lat references for Dual View Binds Same Eid (M2a) and Per Pattern Time Scopes (M1, M2a) are kept as the single owners, no duplicates added.*

## 12. openCypher TCK subset

- [x] 12.1 Vendor the TCK feature files at the commit pinned by `open-cypher` into `crates/tm-cypher/tests/tck/features/`. Add a small Gherkin runner adapted from oxilite's `tests/tck.rs` (no `cucumber` dependency) that runs every scenario, with setup steps through `Db::cypher_write`, a result-table parser for Neo4j notation, and a pass-rate report (overall, read-only, per directory).
- [x] 12.2 Enable the in-scope clause features: `Match1–3`, `Match6` (fixed), `Match7–8`, `MatchWhere1–6`, `Create1–6`, `Merge1–9`, `Set1–6`, `Remove1–3`, `Delete1–6`, `Return1–8`, `ReturnOrderBy1–6`, `ReturnSkipLimit1–3`, `With1–7`, `WithWhere1–7`, `WithOrderBy1–4`, `WithSkipLimit1–3`, `Unwind1`, `Union1–3`.
- [x] 12.3 Enable the in-scope expression features: `Aggregation1–8`, `Boolean1–5`, `Comparison1–4`, `Conditional1–2`, `ExistentialSubquery1–3`, `Graph1–4/6/8/9`, `Literals1–8`, `Map1–3`, `Null1–3`, `Path1–3` (fixed), `Pattern1`, `Precedence1–4`, `String1–14`, `List1–12`, `Mathematical1–17`, `TypeConversion1–6`, and the `date()`/`datetime()`/`localdatetime()` construction and comparison scenarios of `Temporal*` (offsets kept per D20). *Note: TCK features vendored from the open-cypher 0.2.1 crate's spec/vendor snapshot (openCypher 2024.3, commit 677cbafabb8c3c5eed458fd3b1ec0daec8d67d23, the pin of open-cypher) into crates/tm-cypher/tests/tck/features (220 files, 3 880 scenarios, with LICENSE/NOTICE and graphs/). Runner: tests/tck.rs with tests/tck_support/{gherkin,values}.rs, adapted from oxilite. Every feature runs (in-scope and out-of-scope alike); scenarios that fail are allow-listed, so the pass rate is 2 548 of 3 880 (65.7 %), or 95.5 % of the 2 668 scenarios outside the allow-listed families temporal (1 064), variable-length paths (83), Call (50) and pattern comprehension (15).*
- [x] 12.4 Write `tests/tck/allowlist.txt` in oxilite's format, one line per expected-failing scenario, `<feature path> [n][#k] :: <reason>`, with a spec or milestone reference in each reason (`Match4–5`, `Match9`, `Call1–6`, `Pattern2`, `Quantifier*`, `Graph5`, `Graph7`, durations, `time`/`localtime`, printed zone names, empty-node persistence, `id()` values, multi-valued shapes). The runner fails on any unexpected failure (a failing scenario not listed) and on any unexpected pass (a listed scenario that passes), so every deviation stays explicit. Support `TM_TCK_WRITE_ALLOWLIST=<file>` to regenerate the list and `TM_TCK_FILTER=<substring>` to run a subset without the two checks. *Note: allowlist.txt has 1 332 lines with a reason and a spec or milestone reference each; the runner fails on unexpected failures and unexpected passes, and supports TM_TCK_WRITE_ALLOWLIST, TM_TCK_FILTER and TM_TCK_REPORT.*

## 13. Differential suite (needs M2a)

- [x] 13.1 Create the fixture builders in `crates/tiramemsu/tests/differential/fixtures/` (employment graph, parallel edges, two-level layers, superseded fact, cascade, cardinality one, valid-time episodes, all literal types) with a mocked clock. Test "Fixture reproducibility".
- [x] 13.2 Write `normalise.rs`: Cypher values and SPARQL terms → `Canon`, with skolem IRIs `node`/`bnode`/`stmt` to ObjectId, `urn:tiramemsu:tx:<t>` and Cypher `txAdded` to `Tx(t)`, numeric canonicalisation, datetimes as instant plus offset, unbound/null to `Missing`. Unit tests: "Null and unbound compare equal", "Numeric normalisation" and "Datetime offsets survive normalisation".
- [x] 13.3 Write the corpus TOML format and runner: category coverage check (≥ 40 pairs), `bag`/`set` and `ordered`/`unordered` comparison, diff report, and the pending mode without the `sparql` feature. Tests: "Corpus completeness check", "Mismatch report" and "SPARQL front end missing".
- [x] 13.4 Write ≥ 40 equivalence pairs across all categories in `dialect-differential-testing`. `// @lat: [[tests#Query#Differential SPARQL Cypher]]` goes on the runner test.
- [x] 13.5 Add the dual-view pairs ("Supporting belief", "Annotation value through both views"). `// @lat: [[tests#Query#Dual View Binds Same Eid]]` goes on the cross-dialect test (move the reference from 8.1 here once M2a lands, so there is exactly one reference).
- [x] 13.6 Add the temporal pairs ("Per-pattern before and after", "History with transaction numbers", as-of with tx number and instant, valid-at) and the divergence pairs (isomorphism, parallel edges, multi-valued properties).
- [x] 13.7 Once `add-sparql-frontend` is merged, turn on the `sparql` feature in CI and make every pair pass. *Note: Corpus: 49 pairs in crates/tiramemsu/tests/differential/corpus/{basic,temporal,dual,divergence}.toml; runner in tests/cypher_differential.rs with differential/{fixtures,normalise,runner}.rs. The facade gets a default-on cargo feature sparql (SPARQL side of the suite); without it the pairs run Cypher-only and are reported pending. The SPARQL temporal side uses FROM <tm:asOf/…> and SERVICE <tm:asOf/…> as M2a implements them. The existing M2a differential/mod.rs and sparql_differential.rs are unchanged. The Dual View Binds Same Eid @lat reference stays with M2a (sparql_annotations.rs), so it is not duplicated.*

## 14. Documentation and closing

- [x] 14.1 Update `lat.md/query.md` (`Front Ends#Cypher`, `Cypher Dual View`, `Temporal Syntax`) and `lat.md/data-model.md` (`Vocabulary Mapping`, `Nodes and Identity`, `Reserved Namespaces`) with the decisions C1–C22 from design.md, the parser choice (`open-cypher`, with oxilite's vendored parser as the first fallback per D23), and `@lat` code references to the `tm-cypher` modules (`[[crates/tm-cypher/src/vocab.rs#resolve]]` and similar). Record the `DeleteConnectedNode` and `Eval` errors in `lat.md/api.md#Errors`, and `Tx::cypher`/`Db::cypher_write` in `lat.md/api.md#Rust Surface`. Run `lat check` until it passes.
- [x] 14.2 Run `cargo test --workspace`, `cargo clippy --workspace -- -D warnings` and `openspec validate add-cypher-frontend --strict`. All must pass. *Note: Final gates run on this tree: cargo fmt --check, cargo clippy --workspace --all-targets -- -D warnings, PROPTEST_CASES=64 cargo test --workspace, lat check, openspec validate add-cypher-frontend --strict all pass. lat.md changes: query.md (Cypher, Cypher Dual View, Temporal Syntax, openCypher TCK leaf in tests.md), data-model.md (Nodes and Identity, Vocabulary Mapping, Reserved Namespaces), api.md (Rust Surface, Errors).*
