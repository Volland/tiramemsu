Depends on `add-query-ir-and-sql-planner` (M1): IR, views, virtual predicates, SQL codegen, decoding. Through M1 it also depends on `add-core-store` (M0). It can run in parallel with `add-cypher-frontend` (M2b); the only shared items are 11.4 and 11.5. Property-path evaluation is left to `add-path-engine` (M3). Each task is sized at about 2 hours or less and is done when its named test(s) pass.

## 1. Crate setup and parsing

- [x] 1.1 Create `crates/tm-sparql` with the dependencies `spargebra` 0.4.x (feature `sparql-12`), `peg` 0.8 (pinned to spargebra's minor version), `tm-ir` and `tm-core`, and the dev-dependencies `oxttl`, `sparesults` and `insta`. Add it to the workspace. `cargo build -p tm-sparql` passes
  - Note: `oxttl` and `sparesults` are dev-dependencies with `rdf-12` / `sparql-12` (needed to build against oxrdf's `rdf-12`), plus `tiramemsu`, `tempfile` and `toml` for the golden and W3C runners. `peg` is only used for the span downcast attempt (see 1.5).
- [x] 1.2 Add `error.rs` with the `Unsupported` feature-name constants from design D10 and constructors for `Parse { dialect: Sparql, span, msg }`. Add the `Sparql` dialect to the facade error enum if M1 has not
  - Note: M1 had no `Parse` error, so `tm-core` gained `Error::Parse { dialect, span, msg }`, `Dialect` and `Span` (additive); the facade re-exports them.
- [x] 1.3 Add `env.rs` (`Env { base_view, vocab, prefixes, speculative }`) and `parse.rs`: parser construction with the predeclared prefixes (`rdf`, `rdfs`, `xsd`, `sys`, `tm`, `v` = @vocab, the db prefix table). Test: `v:` resolves without `PREFIX`, and a query `PREFIX` overrides it
- [x] 1.4 Implement query-vs-update dispatch (design D2), including the choice of which parser error to report. Test: an update keyword gives the update error, and other text gives the query error
- [x] 1.5 Implement span extraction via `source()` downcast to `peg::error::ParseError<LineCol>`, with the Display fallback (design D3). Test: `SELECT ?s WHERE {\n  ?s v:p ?o\n  FILTER(\n}` reports line 4, column 1, and the byte offset matches
  - Note: `spargebra` 0.4.7 keeps the peg error behind a transparent wrapper whose `source()` chain does not reach it, so the position is read from the `error at L:C` Display text (the downcast is tried first). peg reports the furthest position, one character past the offending one when its catch-all `[_]` is expected, so the span steps back to that character (the spec's line 4, column 1). "Prefix not found" is lost behind a later peg failure, so an undeclared prefix is recovered by scanning the text.

## 2. Terms and values

- [x] 2.1 Add `terms.rs`: spargebra `NamedNode`/`Literal` → IR constant through the M0 codec (canonical encoding). Tests: `"01"^^xsd:integer` equals `1`; `"2026-03-01T12:00:00+02:00"` and `"2026-03-01T10:00:00Z"` get two different ids with the same instant (`id >> 15`), a date-time without a timezone gets `tz` 0, and each decodes to its own lexical offset (design D9, `lat.md/tests#ObjectId#DateTime Keeps Its Offset`); and `"x"@DE` gets the `de` tag
  - Note: the M0 codec already canonicalises literals and offsets (`Value::literal`); `terms.rs` wraps it.
- [x] 2.2 Parse and format the skolem IRIs `urn:tiramemsu:{node,bnode,stmt,tx}:<n>` to and from `NODE`/`BNODE`/`STMT`/`TX` ids. Property test: format → parse round-trips for every tag
- [x] 2.3 Add `lower/vars.rs`: a fresh internal variable allocator whose names cannot collide with SPARQL variables, and a projection filter that hides them from `SELECT *`

## 3. Temporal dataset

- [x] 3.1 Add `dataset.rs`: the `TimeIri` parser for `asOf/<t>`, `asOf/<date|dateTime>`, `validAt/<date|dateTime>` and `history`. Dates mean 00:00Z, and missing timezones mean UTC. Malformed `tm:` IRIs → `Parse`. Add unit tests for each row of the design D7 table
- [x] 3.2 Add `ViewScope` with per-part inheritance (nested `SERVICE <tm:…>`, innermost first > `FROM`/`USING` > API view), conflicting-selector detection, `FROM NAMED` as a no-op, and `Unsupported("named graph")` for non-`tm:` IRIs. Tests follow the `sparql-temporal-dataset` "FROM sets the query default view" scenarios
- [x] 3.3 Integration tests (in `tiramemsu/tests/sparql_temporal.rs`) covering asOf tx, asOf instant (including before the first tx), validAt half-open end, history, nested `SERVICE` (parts combine, innermost wins for the same part, inner overrides `FROM`), `SERVICE SILENT` behaving like `SERVICE`, and the "time IRIs are plain IRIs elsewhere" scenario
  - Note: integration tests live in `crates/tiramemsu/tests/sparql_temporal.rs` and `sparql_temporal_service.rs` (the facade wiring of 7.4 was done first so that they could run).
- [x] 3.4 Per-pattern diff test with `// @lat: [[tests#Query#Per Pattern Time Scopes]]`: a superseded fact returns before/after values from a query that mixes `SERVICE <…asOf/t>` and the now scope, and the same query written with `GRAPH <…asOf/t>` fails with a `Parse` error that names `SERVICE`
- [x] 3.5 Statement-time virtual predicate tests from SPARQL: `tm:txAdded` joined with `sys:author`, `tm:txRetracted` absent on live statements and present under history, and `tm:validFrom`/`tm:validTo` as `xsd:dateTime`

## 4. Pattern lowering

- [x] 4.1 Add `lower/mod.rs`: `Query` → `QueryPlan` for SELECT/ASK/CONSTRUCT with the SPARQL `Semantics` flags, and `DESCRIBE` → `Unsupported`. Add a textual IR printer for golden tests (`insta` snapshots)
- [x] 4.2 Add `lower/pattern.rs` for `Join`, `LeftJoin` (cond), `Filter`, `Union` (flattening), `Extend` and `Values` (UNDEF), with golden IR snapshots
- [x] 4.3 Lower `Service` with a `tm:` IRI (scope push, `silent` ignored, D7), `Service` with another IRI or a variable → `Unsupported("SERVICE")`, `Graph` with a `tm:` IRI → `Parse` naming `SERVICE`, `Graph` with another IRI → `Unsupported("named graph")` (also inside a time `SERVICE`), and `GRAPH ?g` → `Unsupported("GRAPH variable")`, with golden tests
- [x] 4.4 Lower `Minus` as a correlated anti-join with the shared-domain condition, and `EXISTS`/`NOT EXISTS`. If M1's `Expr` lacks `Exists(Op)`, add it to `tm-ir` additively. Tests cover the three `sparql-query` MINUS/EXISTS scenarios
  - Note: M1's `Expr::Exists` already existed. `MINUS` adds the domain condition (some shared variable bound on the right) as a `Bound` filter inside the `NOT EXISTS`; a variable that may be unbound on the left is only counted when every shared variable may be (W3C `partial-minuend` is a listed deviation).
- [x] 4.5 Add `lower/agg.rs`: `Group` → `Aggregate` (COUNT/*, DISTINCT, SUM, AVG, MIN, MAX, SAMPLE, GROUP_CONCAT with separator), HAVING, the implicit single group, and custom aggregate → `Unsupported`
- [x] 4.6 Lower the solution modifiers `Project`/`Distinct`/`Reduced`/`OrderBy`/`Slice` with the D5a fusion and hidden sort columns, plus the DISTINCT + non-projected key → `Unsupported`. Lower subqueries (nested Project scoping)
- [x] 4.7 Add `lower/path.rs` (interim): a single IRI → triple pattern, `^iri` → swapped triple pattern, and every other form → `Unsupported("property path")`. Leave a clearly marked hook for M3
  - Note: `spargebra` desugars a sequence of plain IRIs (`a/b`) into a BGP with a generated blank node, so the algebra cannot tell it from `[]`. The interim requirement (every sequence fails) is enforced by scanning the text (`parse::has_sequence_path`).

## 5. Expressions

- [x] 5.1 Add `lower/expr.rs` for operators, `IN`/`NOT IN`, `BOUND`, `IF`, `COALESCE`, `sameTerm` and the type tests. Golden tests cover the error → false cases (type error, unbound, `!` of an error, `||` rescue)
  - Note: `spargebra` 0.4.7 parses `a - b - c` and `a / b / c` right-associatively. `lower/expr.rs` re-associates chains to the left when the text has no parenthesised operand of that class (`parse::assoc_hints`). `COALESCE()` and `COALESCE(x)` are lowered without the invalid SQL `COALESCE`.
- [x] 5.2 Add the string, numeric and date functions from the whitelist in the `sparql-query` "Built-in functions" requirement, with one scenario test per function family. Date-time comparison and `ORDER BY` use the instant (`id >> 15`, no timezone = UTC); `sameTerm` and joins use the whole id; `YEAR`…`SECONDS` read local time in the stored offset; `TZ` returns the offset (`""` without one) and `TIMEZONE` errors without one. Tests cover the `sparql-query` date-time offset scenarios
  - Note: M1's `Func` had 13 functions, so `tm-ir` gained the missing `Func` variants (additive) and `tm-exec` implements them (new `udf_fn.rs`, `VClass::Lit` for boxed literals of `STRDT`/`STRLANG`/`TIMEZONE`). Limits: string functions drop language tags, `xsd:decimal` results are doubles, `SECONDS` is an integer when whole. Arithmetic on strings is now an error (M1 fix in `num_of`: SQLite coerced text to numbers).
- [x] 5.3 Map XSD constructor casts (`Custom(xsd:…)`) to IR casts, and send other `Custom(iri)`, the non-whitelisted built-ins and the SPARQL 1.2 triple/langdir functions to `Unsupported(<name>)`. Add a table-driven test listing every rejected function name
- [x] 5.4 Make `NOW()` fixed per query at the wall-clock start. Test: two `NOW()` calls in one query are equal

## 6. RDF 1.2 reifiers and annotations (query side)

- [x] 6.1 Add `lower/bgp.rs`: BGP flattening across `Join`s of `Bgp`s within one scope (covers the `DELETE WHERE` shape)
- [x] 6.2 Resolve `r rdf:reifies <<( s p o )>>` to `TriplePattern{eid: r}` (variable, blank → fresh var, statement IRI → constant, other IRI → empty), and triple terms in object position to fresh eid variables, recursively for nesting
- [x] 6.3 Eliminate redundant asserted triples, and reject `rdf:reifies` without a triple term and variable predicates with triple-term objects. Golden IR snapshots for `~ ?r`, `<< … >>`, `{| … |}`, nested annotations and triple terms in object position
- [x] 6.4 Integration tests for every scenario of `sparql-rdf12-annotations` "Reifiers bind statement eids", "Annotation syntax matches layer triples", "Nested layers", "Triple terms denote statements", "rdf:reifies is virtual" and "Reifiers follow the pattern's view"

## 7. Results

- [x] 7.1 Add `results/term.rs`: ObjectId value → RDF term per the design D9 table (dateTime rendered in its stored offset, `Z` for offset 0, no suffix without a timezone, optional `.sss`; lower-cased language tags; skolem IRIs)
- [x] 7.2 Add `results/json.rs`: a SPARQL 1.1 Query Results JSON writer (vars in projection order, unbound omitted, ASK document). Validate the output by parsing it back with `sparesults` in tests
- [x] 7.3 Add `construct.rs` + `results/nt.rs`: template instantiation (skip unbound or invalid positions, fresh blank nodes per solution, set dedup), reifier/annotation templates emitting `rdf:reifies <<( … )>>`, and the N-Triples / RDF 1.2 N-Triples writer
- [x] 7.4 In the facade: `View::sparql`, the `QueryResult` variants `Boolean`/`Graph`/`Update(TxReport)`, and the `write_sparql_json` / `write_ntriples` methods. Queries run on the view's connection (the writer inside `with`)
  - Note: `QueryResult` is M1's struct, so the facade adds a separate enum `SparqlResult { Solutions, Boolean, Graph, Update }` returned by `View::sparql`, with `write_sparql_json` and `write_ntriples`. `tm-sparql` does not depend on `tm-exec`; the facade converts rows. `tm-core` gained `Tx::read_with` (read on the transaction's connection) for the update `WHERE`.

## 8. Updates

- [x] 8.1 Add `update.rs`: `UpdatePlan` built from `GraphUpdateOperation`s. `LOAD`/`CLEAR`/`CREATE`/`DROP` → `Unsupported(<KEYWORD>)`, and `GRAPH` in data or templates, whatever the IRI (including desugared `WITH`/`ADD`/`MOVE`/`COPY`) → `Unsupported("named graph")`, all rejected before the transaction opens. `GRAPH`/`SERVICE` inside `WHERE` go through the query lowering (4.3)
- [x] 8.2 Reject updates unless the view is the plain current, non-speculative view. Execute the whole request in one `db.transact`, and return `QueryResult::Update(TxReport)`. Tests: multi-operation request with one `t`, failure leaves no trace, no-op request still commits
  - Note: the update runner is `tm_sparql::update::run`; the facade executes it inside `Db::transact`.
- [x] 8.3 `INSERT DATA` → assert (unbounded valid time, fresh BNODE per label per request), and `DELETE DATA` → `retract_matching` with cascade. Tests: idempotent re-insert reports existing, all episodes retracted, annotations cascaded, as-of before still visible
- [x] 8.4 `DELETE/INSERT WHERE` and `DELETE WHERE`: evaluate `WHERE` on the writer inside the tx (time scopes and `USING` honoured), apply deletes before inserts, and use fresh blank nodes per solution. Tests cover the five `sparql-update` DELETE/INSERT WHERE scenarios, the restore-from-asOf scenario (`SERVICE <…asOf/150>` in `WHERE`), and `GRAPH <tm:…>` in `WHERE` failing with `Parse`
- [x] 8.5 Reifiers in updates: σ from reifier to asserted eid, annotations asserted on the eid, reified triple / triple term asserted on insert, eid-precise `retract(eid)` for bound reifiers in delete templates, and the reifier restrictions (`reifier that is not a statement`, `reifier of more than one triple`). Tests cover every `sparql-rdf12-annotations` update scenario
- [x] 8.6 Reject tm: statement-time predicates and `sys:` virtual hops in inserts with `ReservedNamespace`. Pass-through tests for `UniqueViolation`, cardinality-one replacement in the report, a schema flag insertable via SPARQL, and `ReservedNamespace` for `sys:supersedes`

## 9. Golden tests

- [x] 9.1 Create the `tests/golden/` layout (`*.rq`/`*.ru` input, `*.ir` snapshot, fixture `*.trig` or API-built fixture, `*.srj` expected results) and a runner that executes every case against a fresh temp database
  - Note: the runner is `crates/tm-sparql/tests/golden.rs`; fixtures are Turtle (loaded through `INSERT DATA`) or named API fixtures. `UPDATE_GOLDEN=1` rewrites the goldens.
- [x] 9.2 Add at least one golden case per `sparql-query` requirement: set-of-triples dedup over episodes, homomorphism, canonical literals, blank nodes, skolem round trip, SELECT JSON with unbound, and ASK JSON
- [x] 9.3 Add error golden cases: every `Unsupported` feature string from design D10 and the parse-error spans, each asserting that no SQL was executed (M1 executor call counter)
  - Note: `custom aggregate` cannot be produced from text (spargebra only parses custom aggregates it was told about) and is asserted as a constant; `update on a non-current view` needs a view, so it is tested in `sparql_update_tx.rs`.

## 10. W3C test-suite subset

- [x] 10.1 Write the manifest-driven runner (`tests/w3c/`): load `manifest.ttl` with `oxttl`, load `qt:data` into a fresh db through the API (Turtle → assert), run the query, parse `.srx`/`.srj` with `sparesults`, and compare with blank-node isomorphism after mapping `urn:tiramemsu:bnode:` IRIs back to blank nodes
- [x] 10.2 Put these SPARQL 1.1 manifest categories in scope: `syntax-query`, `syntax-update-1`, `aggregates`, `bind`, `bindings`, `construct`, `exists`, `functions` (whitelisted functions only), `grouping`, `negation`, `project-expression`, `subquery`, `json-res`, `basic-update`, `delete-data`, `delete-insert`, `delete-where`, `insert-data` (default-graph cases only). SPARQL 1.0 `data-r2`: `basic`, `triple-match`, `optional`, `optional-filter`, `algebra`, `bound`, `distinct`, `expr-builtin`, `expr-equals`, `expr-ops`, `open-world`, `regex`, `solution-seq`, `sort`, `ask`, `construct`, `boolean-effective-value`, `type-promotion`. SPARQL 1.2: `syntax-triple-terms-positive`/`-negative` and the reifier/annotation evaluation tests
  - Note: the `insert-data` category does not exist in the suite (INSERT DATA is covered by `basic-update`); the data is copied under `tests/w3c/data` (6.3 MB, with `LICENSE` and `SOURCE.md`). Tests with named graphs, `.rdf` data or results (`oxrdfxml` does not build with `rdf-12`) are skipped and counted in the summary.
- [x] 10.3 Mark as out of scope, and do not run: `property-path` (until M3), `service`, `entailment`, `protocol`, `http-rdf-update`, `csv-tsv-res`, `clear`, `drop`, `copy`, `move`, `add`, `dataset`, `graph`, and the named-graph cases of the update manifests
- [x] 10.4 Add `expected-deviations.toml` (test IRI + reason: canonical integers and other numbers, sub-millisecond date-time digits truncated, lower-cased language tags, skolemised blank nodes in results, asserted reifications, rejected functions). The runner fails on any unlisted failure **and** on any listed test that now passes. Wire it into CI
  - Note: wired into CI as an explicit step; `W3C_REPORT=1`, `W3C_ONLY=<substring>` and `W3C_WRITE_DEVIATIONS=1` help triage.

## 11. Cross-dialect and documentation

- [x] 11.1 Add a facade API test: an eid from the API `assert` equals the eid bound by `~ ?r`, and `<urn:tiramemsu:stmt:N>` reads its annotations
- [x] 11.2 Add the MCP-facing smoke test: `View::sparql` + `write_sparql_json` on the current and `as_of` views produce valid JSON documents
- [x] 11.3 Benchmark hook: add a SPARQL variant of the M1 point and 2-hop latency benchmarks (now/asOf/validAt) to `roadmap#Benchmarks`, using the SPARQL front end
  - Note: M1 had no point / 2-hop benchmarks, so `benches/sparql.rs` adds them for the SPARQL front end (now, asOf, validAt); `roadmap#Benchmarks` mentions it.
- [x] 11.4 Coordinate with `add-cypher-frontend`: contribute the SPARQL side of the differential corpus (`tests#Query#Differential SPARQL Cypher`; the `@lat` comment lives in M2b's harness) and the SPARQL assertion in the dual-view test. Add `// @lat: [[tests#Query#Dual View Binds Same Eid]]` only if M2b has not already placed it
  - Note: the corpus is `crates/tiramemsu/tests/differential/mod.rs` with the SPARQL text and the expected rows per case; M2b adds the Cypher text. The Dual View `@lat` reference is placed in `sparql_annotations.rs` (M2b had not placed it).
- [x] 11.5 Update lat.md (`query.md`, `Front Ends#SPARQL` and `Temporal Syntax`) with `@lat` code refs to `tm-sparql` (`// @lat: [[query#Front Ends#SPARQL]]` in `lower/mod.rs`, `// @lat: [[query#Temporal Syntax]]` in `dataset.rs`). Record the decisions chosen during spec writing (skolem `stmt:`/`tx:` forms, predeclared prefixes, reified triples asserted on insert, update only on the current view) in `lat.md/query.md` and `lat.md/data-model.md`. Run `lat check` until it passes
