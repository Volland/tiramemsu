Depends on `add-query-ir-and-sql-planner` (M1): IR, views, virtual predicates, SQL codegen, decoding. Through M1 it also depends on `add-core-store` (M0). It can run in parallel with `add-cypher-frontend` (M2b); the only shared items are 11.4 and 11.5. Property-path evaluation is left to `add-path-engine` (M3). Each task is sized at about 2 hours or less and is done when its named test(s) pass.

## 1. Crate setup and parsing

- [ ] 1.1 Create `crates/tm-sparql` with the dependencies `spargebra` 0.4.x (feature `sparql-12`), `peg` 0.8 (pinned to spargebra's minor version), `tm-ir` and `tm-core`, and the dev-dependencies `oxttl`, `sparesults` and `insta`. Add it to the workspace. `cargo build -p tm-sparql` passes
- [ ] 1.2 Add `error.rs` with the `Unsupported` feature-name constants from design D10 and constructors for `Parse { dialect: Sparql, span, msg }`. Add the `Sparql` dialect to the facade error enum if M1 has not
- [ ] 1.3 Add `env.rs` (`Env { base_view, vocab, prefixes, speculative }`) and `parse.rs`: parser construction with the predeclared prefixes (`rdf`, `rdfs`, `xsd`, `sys`, `tm`, `v` = @vocab, the db prefix table). Test: `v:` resolves without `PREFIX`, and a query `PREFIX` overrides it
- [ ] 1.4 Implement query-vs-update dispatch (design D2), including the choice of which parser error to report. Test: an update keyword gives the update error, and other text gives the query error
- [ ] 1.5 Implement span extraction via `source()` downcast to `peg::error::ParseError<LineCol>`, with the Display fallback (design D3). Test: `SELECT ?s WHERE {\n  ?s v:p ?o\n  FILTER(\n}` reports line 4, column 1, and the byte offset matches

## 2. Terms and values

- [ ] 2.1 Add `terms.rs`: spargebra `NamedNode`/`Literal` → IR constant through the M0 codec (canonical encoding). Tests: `"01"^^xsd:integer` equals `1`; `"2026-03-01T12:00:00+02:00"` and `"2026-03-01T10:00:00Z"` get two different ids with the same instant (`id >> 15`), a date-time without a timezone gets `tz` 0, and each decodes to its own lexical offset (design D9, `lat.md/tests#ObjectId#DateTime Keeps Its Offset`); and `"x"@DE` gets the `de` tag
- [ ] 2.2 Parse and format the skolem IRIs `urn:tiramemsu:{node,bnode,stmt,tx}:<n>` to and from `NODE`/`BNODE`/`STMT`/`TX` ids. Property test: format → parse round-trips for every tag
- [ ] 2.3 Add `lower/vars.rs`: a fresh internal variable allocator whose names cannot collide with SPARQL variables, and a projection filter that hides them from `SELECT *`

## 3. Temporal dataset

- [ ] 3.1 Add `dataset.rs`: the `TimeIri` parser for `asOf/<t>`, `asOf/<date|dateTime>`, `validAt/<date|dateTime>` and `history`. Dates mean 00:00Z, and missing timezones mean UTC. Malformed `tm:` IRIs → `Parse`. Add unit tests for each row of the design D7 table
- [ ] 3.2 Add `ViewScope` with per-part inheritance (nested `SERVICE <tm:…>`, innermost first > `FROM`/`USING` > API view), conflicting-selector detection, `FROM NAMED` as a no-op, and `Unsupported("named graph")` for non-`tm:` IRIs. Tests follow the `sparql-temporal-dataset` "FROM sets the query default view" scenarios
- [ ] 3.3 Integration tests (in `tiramemsu/tests/sparql_temporal.rs`) covering asOf tx, asOf instant (including before the first tx), validAt half-open end, history, nested `SERVICE` (parts combine, innermost wins for the same part, inner overrides `FROM`), `SERVICE SILENT` behaving like `SERVICE`, and the "time IRIs are plain IRIs elsewhere" scenario
- [ ] 3.4 Per-pattern diff test with `// @lat: [[tests#Query#Per Pattern Time Scopes]]`: a superseded fact returns before/after values from a query that mixes `SERVICE <…asOf/t>` and the now scope, and the same query written with `GRAPH <…asOf/t>` fails with a `Parse` error that names `SERVICE`
- [ ] 3.5 Statement-time virtual predicate tests from SPARQL: `tm:txAdded` joined with `sys:author`, `tm:txRetracted` absent on live statements and present under history, and `tm:validFrom`/`tm:validTo` as `xsd:dateTime`

## 4. Pattern lowering

- [ ] 4.1 Add `lower/mod.rs`: `Query` → `QueryPlan` for SELECT/ASK/CONSTRUCT with the SPARQL `Semantics` flags, and `DESCRIBE` → `Unsupported`. Add a textual IR printer for golden tests (`insta` snapshots)
- [ ] 4.2 Add `lower/pattern.rs` for `Join`, `LeftJoin` (cond), `Filter`, `Union` (flattening), `Extend` and `Values` (UNDEF), with golden IR snapshots
- [ ] 4.3 Lower `Service` with a `tm:` IRI (scope push, `silent` ignored, D7), `Service` with another IRI or a variable → `Unsupported("SERVICE")`, `Graph` with a `tm:` IRI → `Parse` naming `SERVICE`, `Graph` with another IRI → `Unsupported("named graph")` (also inside a time `SERVICE`), and `GRAPH ?g` → `Unsupported("GRAPH variable")`, with golden tests
- [ ] 4.4 Lower `Minus` as a correlated anti-join with the shared-domain condition, and `EXISTS`/`NOT EXISTS`. If M1's `Expr` lacks `Exists(Op)`, add it to `tm-ir` additively. Tests cover the three `sparql-query` MINUS/EXISTS scenarios
- [ ] 4.5 Add `lower/agg.rs`: `Group` → `Aggregate` (COUNT/*, DISTINCT, SUM, AVG, MIN, MAX, SAMPLE, GROUP_CONCAT with separator), HAVING, the implicit single group, and custom aggregate → `Unsupported`
- [ ] 4.6 Lower the solution modifiers `Project`/`Distinct`/`Reduced`/`OrderBy`/`Slice` with the D5a fusion and hidden sort columns, plus the DISTINCT + non-projected key → `Unsupported`. Lower subqueries (nested Project scoping)
- [ ] 4.7 Add `lower/path.rs` (interim): a single IRI → triple pattern, `^iri` → swapped triple pattern, and every other form → `Unsupported("property path")`. Leave a clearly marked hook for M3

## 5. Expressions

- [ ] 5.1 Add `lower/expr.rs` for operators, `IN`/`NOT IN`, `BOUND`, `IF`, `COALESCE`, `sameTerm` and the type tests. Golden tests cover the error → false cases (type error, unbound, `!` of an error, `||` rescue)
- [ ] 5.2 Add the string, numeric and date functions from the whitelist in the `sparql-query` "Built-in functions" requirement, with one scenario test per function family. Date-time comparison and `ORDER BY` use the instant (`id >> 15`, no timezone = UTC); `sameTerm` and joins use the whole id; `YEAR`…`SECONDS` read local time in the stored offset; `TZ` returns the offset (`""` without one) and `TIMEZONE` errors without one. Tests cover the `sparql-query` date-time offset scenarios
- [ ] 5.3 Map XSD constructor casts (`Custom(xsd:…)`) to IR casts, and send other `Custom(iri)`, the non-whitelisted built-ins and the SPARQL 1.2 triple/langdir functions to `Unsupported(<name>)`. Add a table-driven test listing every rejected function name
- [ ] 5.4 Make `NOW()` fixed per query at the wall-clock start. Test: two `NOW()` calls in one query are equal

## 6. RDF 1.2 reifiers and annotations (query side)

- [ ] 6.1 Add `lower/bgp.rs`: BGP flattening across `Join`s of `Bgp`s within one scope (covers the `DELETE WHERE` shape)
- [ ] 6.2 Resolve `r rdf:reifies <<( s p o )>>` to `TriplePattern{eid: r}` (variable, blank → fresh var, statement IRI → constant, other IRI → empty), and triple terms in object position to fresh eid variables, recursively for nesting
- [ ] 6.3 Eliminate redundant asserted triples, and reject `rdf:reifies` without a triple term and variable predicates with triple-term objects. Golden IR snapshots for `~ ?r`, `<< … >>`, `{| … |}`, nested annotations and triple terms in object position
- [ ] 6.4 Integration tests for every scenario of `sparql-rdf12-annotations` "Reifiers bind statement eids", "Annotation syntax matches layer triples", "Nested layers", "Triple terms denote statements", "rdf:reifies is virtual" and "Reifiers follow the pattern's view"

## 7. Results

- [ ] 7.1 Add `results/term.rs`: ObjectId value → RDF term per the design D9 table (dateTime rendered in its stored offset, `Z` for offset 0, no suffix without a timezone, optional `.sss`; lower-cased language tags; skolem IRIs)
- [ ] 7.2 Add `results/json.rs`: a SPARQL 1.1 Query Results JSON writer (vars in projection order, unbound omitted, ASK document). Validate the output by parsing it back with `sparesults` in tests
- [ ] 7.3 Add `construct.rs` + `results/nt.rs`: template instantiation (skip unbound or invalid positions, fresh blank nodes per solution, set dedup), reifier/annotation templates emitting `rdf:reifies <<( … )>>`, and the N-Triples / RDF 1.2 N-Triples writer
- [ ] 7.4 In the facade: `View::sparql`, the `QueryResult` variants `Boolean`/`Graph`/`Update(TxReport)`, and the `write_sparql_json` / `write_ntriples` methods. Queries run on the view's connection (the writer inside `with`)

## 8. Updates

- [ ] 8.1 Add `update.rs`: `UpdatePlan` built from `GraphUpdateOperation`s. `LOAD`/`CLEAR`/`CREATE`/`DROP` → `Unsupported(<KEYWORD>)`, and `GRAPH` in data or templates, whatever the IRI (including desugared `WITH`/`ADD`/`MOVE`/`COPY`) → `Unsupported("named graph")`, all rejected before the transaction opens. `GRAPH`/`SERVICE` inside `WHERE` go through the query lowering (4.3)
- [ ] 8.2 Reject updates unless the view is the plain current, non-speculative view. Execute the whole request in one `db.transact`, and return `QueryResult::Update(TxReport)`. Tests: multi-operation request with one `t`, failure leaves no trace, no-op request still commits
- [ ] 8.3 `INSERT DATA` → assert (unbounded valid time, fresh BNODE per label per request), and `DELETE DATA` → `retract_matching` with cascade. Tests: idempotent re-insert reports existing, all episodes retracted, annotations cascaded, as-of before still visible
- [ ] 8.4 `DELETE/INSERT WHERE` and `DELETE WHERE`: evaluate `WHERE` on the writer inside the tx (time scopes and `USING` honoured), apply deletes before inserts, and use fresh blank nodes per solution. Tests cover the five `sparql-update` DELETE/INSERT WHERE scenarios, the restore-from-asOf scenario (`SERVICE <…asOf/150>` in `WHERE`), and `GRAPH <tm:…>` in `WHERE` failing with `Parse`
- [ ] 8.5 Reifiers in updates: σ from reifier to asserted eid, annotations asserted on the eid, reified triple / triple term asserted on insert, eid-precise `retract(eid)` for bound reifiers in delete templates, and the reifier restrictions (`reifier that is not a statement`, `reifier of more than one triple`). Tests cover every `sparql-rdf12-annotations` update scenario
- [ ] 8.6 Reject tm: statement-time predicates and `sys:` virtual hops in inserts with `ReservedNamespace`. Pass-through tests for `UniqueViolation`, cardinality-one replacement in the report, a schema flag insertable via SPARQL, and `ReservedNamespace` for `sys:supersedes`

## 9. Golden tests

- [ ] 9.1 Create the `tests/golden/` layout (`*.rq`/`*.ru` input, `*.ir` snapshot, fixture `*.trig` or API-built fixture, `*.srj` expected results) and a runner that executes every case against a fresh temp database
- [ ] 9.2 Add at least one golden case per `sparql-query` requirement: set-of-triples dedup over episodes, homomorphism, canonical literals, blank nodes, skolem round trip, SELECT JSON with unbound, and ASK JSON
- [ ] 9.3 Add error golden cases: every `Unsupported` feature string from design D10 and the parse-error spans, each asserting that no SQL was executed (M1 executor call counter)

## 10. W3C test-suite subset

- [ ] 10.1 Write the manifest-driven runner (`tests/w3c/`): load `manifest.ttl` with `oxttl`, load `qt:data` into a fresh db through the API (Turtle → assert), run the query, parse `.srx`/`.srj` with `sparesults`, and compare with blank-node isomorphism after mapping `urn:tiramemsu:bnode:` IRIs back to blank nodes
- [ ] 10.2 Put these SPARQL 1.1 manifest categories in scope: `syntax-query`, `syntax-update-1`, `aggregates`, `bind`, `bindings`, `construct`, `exists`, `functions` (whitelisted functions only), `grouping`, `negation`, `project-expression`, `subquery`, `json-res`, `basic-update`, `delete-data`, `delete-insert`, `delete-where`, `insert-data` (default-graph cases only). SPARQL 1.0 `data-r2`: `basic`, `triple-match`, `optional`, `optional-filter`, `algebra`, `bound`, `distinct`, `expr-builtin`, `expr-equals`, `expr-ops`, `open-world`, `regex`, `solution-seq`, `sort`, `ask`, `construct`, `boolean-effective-value`, `type-promotion`. SPARQL 1.2: `syntax-triple-terms-positive`/`-negative` and the reifier/annotation evaluation tests
- [ ] 10.3 Mark as out of scope, and do not run: `property-path` (until M3), `service`, `entailment`, `protocol`, `http-rdf-update`, `csv-tsv-res`, `clear`, `drop`, `copy`, `move`, `add`, `dataset`, `graph`, and the named-graph cases of the update manifests
- [ ] 10.4 Add `expected-deviations.toml` (test IRI + reason: canonical integers and other numbers, sub-millisecond date-time digits truncated, lower-cased language tags, skolemised blank nodes in results, asserted reifications, rejected functions). The runner fails on any unlisted failure **and** on any listed test that now passes. Wire it into CI

## 11. Cross-dialect and documentation

- [ ] 11.1 Add a facade API test: an eid from the API `assert` equals the eid bound by `~ ?r`, and `<urn:tiramemsu:stmt:N>` reads its annotations
- [ ] 11.2 Add the MCP-facing smoke test: `View::sparql` + `write_sparql_json` on the current and `as_of` views produce valid JSON documents
- [ ] 11.3 Benchmark hook: add a SPARQL variant of the M1 point and 2-hop latency benchmarks (now/asOf/validAt) to `roadmap#Benchmarks`, using the SPARQL front end
- [ ] 11.4 Coordinate with `add-cypher-frontend`: contribute the SPARQL side of the differential corpus (`tests#Query#Differential SPARQL Cypher`; the `@lat` comment lives in M2b's harness) and the SPARQL assertion in the dual-view test. Add `// @lat: [[tests#Query#Dual View Binds Same Eid]]` only if M2b has not already placed it
- [ ] 11.5 Update lat.md (`query.md`, `Front Ends#SPARQL` and `Temporal Syntax`) with `@lat` code refs to `tm-sparql` (`// @lat: [[query#Front Ends#SPARQL]]` in `lower/mod.rs`, `// @lat: [[query#Temporal Syntax]]` in `dataset.rs`). Record the decisions chosen during spec writing (skolem `stmt:`/`tx:` forms, predeclared prefixes, reified triples asserted on insert, update only on the current view) in `lat.md/query.md` and `lat.md/data-model.md`. Run `lat check` until it passes
