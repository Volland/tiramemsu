## Why

Tiramemsu promises that SPARQL and Cypher query one store with the same results (`lat.md/query#Front Ends`, decision D13). After M1 (`add-query-ir-and-sql-planner`) there is a logical IR and a SQL planner, but no language reaches them. RDF users and LLM agents that speak SPARQL cannot yet read or write the store. They also cannot reach the two things that make Tiramemsu different: statement identity (eids) through SPARQL 1.2 reifiers and annotations, and time travel through the `tm:` time IRIs (`lat.md/query#Temporal Syntax`). This is milestone M2a (`lat.md/roadmap#Milestones`). It can start as soon as M1 lands, and it runs in parallel with `add-cypher-frontend` (M2b).

## What Changes

- Add a new crate, **`tm-sparql`**. It parses SPARQL 1.1 plus SPARQL 1.2 triple terms, reifiers and annotations with `spargebra` (feature `sparql-12`), and lowers the algebra to the M1 IR with the SPARQL semantic flags: `graph_set = SetOfTriples`, `match_mode = Homomorphism`, `missing = Unbound`.
- Add **`View::sparql(text)`** to the `tiramemsu` facade (`lat.md/api#Rust Surface`). It runs `SELECT`, `ASK` and `CONSTRUCT` against the view. It also accepts SPARQL Update on the current view and runs the whole request as one transaction.
- **v1 query subset:** BGP, `OPTIONAL`, `FILTER` (an error makes the filter false), `UNION`, `MINUS`, `EXISTS`/`NOT EXISTS`, `BIND`, `VALUES`, aggregates with `GROUP BY`/`HAVING`, subqueries, and `ORDER BY`/`LIMIT`/`OFFSET`/`DISTINCT`/`REDUCED`. A documented set of built-in functions is included.
- **Property paths** are parsed. A path that is a single IRI or its inverse is evaluated as a triple pattern. Every other path form fails with `Unsupported { feature: "property path" }` until `add-path-engine` (M3) replaces that requirement.
- **v1 update:** `INSERT DATA` maps to idempotent assert. `DELETE DATA` maps to retract of every live matching eid, with cascade. `DELETE/INSERT … WHERE` (and `DELETE WHERE`) evaluates once and applies in one transaction, and the transaction report (with `t`) is returned (`lat.md/time-model#Operations`).
- **SPARQL 1.2:** reifiers (`~ ?r`, `<< s p o ~ ?r >>`) and triple terms (`<<( s p o )>>`) bind directly to eids. Annotations (`{| … |}`) read and write layer triples whose subject is the eid, nested to any depth (`lat.md/data-model#Layers`). `rdf:reifies` is virtual and never stored.
- **Time IRIs** in `FROM` and `GRAPH`: `tm:asOf/<t>`, `tm:asOf/<dateTime>`, `tm:validAt/<date|dateTime>` and `tm:history`. `FROM` sets the query default, and `GRAPH` scopes single patterns. Statement-time virtual predicates (`tm:txAdded`, `tm:txRetracted`, `tm:validFrom`, `tm:validTo`, `tm:retractKind`) are usable in patterns. `tm:` IRIs anywhere else are plain IRIs.
- **Results:** typed solutions, with serialisation to SPARQL 1.1 Query Results JSON for `SELECT`/`ASK` and to N-Triples (RDF 1.2 when triple terms occur) for `CONSTRUCT`. Nodes, blank nodes, eids and transactions are rendered as round-trippable skolem IRIs.
- **Errors:** `Parse { dialect: Sparql, span, msg }` with line and column, and `Unsupported { feature }` naming the construct (`lat.md/api#Errors`). Features outside v1 (DESCRIBE, SERVICE, named graphs, LOAD/CLEAR/CREATE/DROP/ADD/MOVE/COPY, custom functions and similar) fail before any execution.
- Add test infrastructure: golden query tests, a W3C SPARQL 1.1/1.2 test-suite subset runner with a recorded list of expected deviations, and the SPARQL half of the differential and dual-view suites.

## Capabilities

### New Capabilities
- `sparql-query`: query forms, the supported algebra subset and built-in functions, SPARQL semantics over the store (set of triples, homomorphism, unbound values, filter errors), literal canonicalisation effects, interim property-path behaviour, parse and unsupported errors, result terms and the result formats.
- `sparql-update`: update forms and how they map to transactions (assert, retract with cascade, one transaction per request, report), template instantiation, time-scoped `WHERE`, constraint enforcement, and unsupported update operations.
- `sparql-rdf12-annotations`: triple terms, reifiers bound to eids, annotation syntax for reading and writing layer triples, nesting, the virtual `rdf:reifies`, reifier rules in updates, and RDF 1.2 output from `CONSTRUCT`.
- `sparql-temporal-dataset`: recognition of the `tm:` time IRIs in `FROM`/`GRAPH`, query defaults and per-pattern scopes, how they combine with the API view, the plain-IRI rule elsewhere, and the statement-time virtual predicates as used from SPARQL.

### Modified Capabilities
- None. `openspec/specs/` holds no archived capability yet. The interim property-path requirement in `sparql-query` is meant to be modified by `add-path-engine` when that change is archived.

## Impact

- **Dependencies:** requires `add-query-ir-and-sql-planner` (M1), which provides the IR, views, virtual predicates, SQL codegen and ObjectId decoding, and through it `add-core-store` (M0), which provides the tx engine, cascade, schema checks and codec. It is independent of `add-cypher-frontend` (M2b), and both can be built in parallel. The shared differential suite (`tests#Query#Differential SPARQL Cypher`) is finished when both exist. `add-path-engine` (M3) later replaces the interim property-path requirement.
- **New crate:** `tm-sparql`, depending on `spargebra` (feature `sparql-12`), `tm-ir` and `tm-core`, plus `peg` for error positions. Dev-dependencies are `oxttl` (Turtle fixtures for the W3C runner) and `sparesults` (expected-result parsing).
- **Facade (`tiramemsu`):** `View::sparql`, the SPARQL result types and serialisers, and a `QueryResult` variant that carries the `TxReport` of an update.
- **Errors:** `Parse` gains the `Sparql` dialect. `Unsupported { feature }` is used with the SPARQL feature names listed in the specs.
- **Docs:** `lat.md/query.md` (`Front Ends#SPARQL`, `Temporal Syntax`) gains `@lat` code references, and `lat check` must pass. The specs `tests#Query#Per Pattern Time Scopes` and `tests#Query#Dual View Binds Same Eid` (SPARQL half) get implementations.
