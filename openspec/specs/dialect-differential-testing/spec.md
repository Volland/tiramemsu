# dialect-differential-testing Specification

## Purpose
Defines the cross-dialect differential test suite. Equivalent SPARQL and Cypher queries run over shared fixtures and must return identical normalised results. This is how Tiramemsu enforces that both front ends mean the same thing over one store, including layers and time. The suite needs both the Cypher and the SPARQL front ends.

## Requirements

### Requirement: Shared fixtures loaded through the API
The suite SHALL define named fixtures as sequences of transactions that are applied through the Rust transaction API (assert, create, retract, supersede, confirm, and tx metadata), never through either query dialect. Each fixture SHALL run against a fresh database, with a deterministic clock so that transaction numbers and instants are reproducible. The fixtures SHALL cover at least: a small social and employment graph with labels and literal properties; parallel edges made by `create`; layers (annotations on statements, statements referencing statements, two levels deep); a superseded fact; a retraction with cascade; a cardinality-one replacement; valid-time episodes; and values of every literal type.

#### Scenario: Fixture reproducibility
- **WHEN** the same fixture is loaded twice into fresh databases
- **THEN** both databases hold identical triple and tx tables (eids, `t_add`, `t_ret`, instants)

### Requirement: Query pair corpus
The suite SHALL hold a corpus of query pairs. Each pair has a SPARQL query, a Cypher query, a fixture name, a comparison mode (`bag` or `set`), an ordering mode (`ordered` or `unordered`), a column mapping, and an optional view (the API time selection applied to both). The corpus SHALL contain at least 40 pairs, spread over: single and multi-hop patterns; labels versus `rdf:type`; literal property filters and comparisons; `OPTIONAL MATCH` versus `OPTIONAL`; `EXISTS`/`NOT EXISTS` versus `FILTER EXISTS`/`FILTER NOT EXISTS`; `UNION`; aggregation with grouping; `ORDER BY` with `SKIP`/`LIMIT`; subqueries; `USE AS OF` versus `FROM <urn:tiramemsu:tm:asOf/…>` (with a tx number and with an instant); `USE VALID AT` versus `FROM <urn:tiramemsu:tm:validAt/…>`; `USE HISTORY` versus `FROM <urn:tiramemsu:tm:history>`; per-pattern `CALL { USE … }` versus `SERVICE <urn:tiramemsu:tm:asOf/…> { … }`; statement time properties versus `tm:txAdded`/`tm:validFrom`; and the dual view versus reifiers.

#### Scenario: Corpus completeness check
- **WHEN** the suite starts
- **THEN** it verifies that the corpus has at least 40 pairs and at least one pair per listed category, and fails with the names of any missing categories otherwise

#### Scenario: Simple pair passes
- **WHEN** the pair SPARQL `SELECT ?c WHERE { ?a a v:Person ; v:worksAt ?c }` and Cypher `MATCH (a:Person)-[:worksAt]->(c) RETURN c` runs on the employment fixture in `bag`/`unordered` mode
- **THEN** both normalised results are equal multisets

### Requirement: Result normalisation
Before comparing, the suite SHALL normalise both results to rows of canonical values:
- IRIs, anonymous nodes, blank nodes and statements become their storage identifier, whether they come as a Cypher Node, a Relationship, an element id string, or a SPARQL term. This includes the skolem IRIs `urn:tiramemsu:node:<n>`, `urn:tiramemsu:bnode:<n>` and `urn:tiramemsu:stmt:<n>`, which parse back to the same identifier.
- Literals become their canonical stored value (numbers compared by numeric value; datetimes as instant plus timezone offset, or no timezone, so an offset dropped or changed by either dialect is a mismatch).
- A SPARQL unbound value and a Cypher `null` become the same missing marker.
- Transactions become their number `t`, whether they come as a SPARQL `urn:tiramemsu:tx:<t>` skolem IRI (for example from `tm:txAdded`) or as the Cypher Integer from `txAdded`.
- Columns are matched through the pair's column mapping.

In `set` mode, duplicate rows SHALL be removed from both sides. In `unordered` mode, rows SHALL be compared as multisets. In `ordered` mode, row order SHALL be compared too, for the sort keys only.

#### Scenario: Null and unbound compare equal
- **WHEN** a pair uses SPARQL `OPTIONAL { ?p v:age ?age }` and Cypher `OPTIONAL MATCH` returning `p.age AS age` for a person without an age
- **THEN** both rows normalise to the missing marker in column `age`, and the pair passes

#### Scenario: Numeric normalisation
- **WHEN** SPARQL returns `"42"^^xsd:integer` and Cypher returns Integer 42 for the same cell
- **THEN** the cells compare equal

#### Scenario: Datetime offsets survive normalisation
- **WHEN** SPARQL returns `"2026-03-01T12:00:00+02:00"^^xsd:dateTime` and Cypher returns DateTime 2026-03-01T12:00:00.000+02:00 for the same cell
- **THEN** the cells compare equal
- **AND** a Cypher DateTime 2026-03-01T10:00:00.000Z in that cell, the same instant with another offset, is reported as a mismatch

### Requirement: Dual view binds the same eid as SPARQL reifiers
For every fact in the layers fixture, a Cypher relationship variable used in node position and a SPARQL reifier `~ ?r` on the same triple SHALL bind the same statement eid.

#### Scenario: Supporting belief
- **WHEN** Cypher `MATCH (a)-[r:WORKS_AT]->(c), (b:Belief)-[:SUPPORTED_BY]->(r) RETURN r, b` and SPARQL `SELECT ?r ?b WHERE { ?a v:WORKS_AT ?c ~ ?r . ?b a v:Belief ; v:SUPPORTED_BY ?r }` run on the layers fixture
- **THEN** the normalised `r` columns are identical, and so are the `b` columns

#### Scenario: Annotation value through both views
- **WHEN** Cypher `MATCH ()-[r:WORKS_AT]->() RETURN r, r.confidence AS conf` and SPARQL `SELECT ?r ?conf WHERE { ?a v:WORKS_AT ?c ~ ?r . ?r v:confidence ?conf }` run
- **THEN** the results are equal

### Requirement: Temporal equivalence
Pairs that use the time syntax of both dialects SHALL return equal results. This covers a query-level as-of with a tx number and with an instant, valid-at, history with statement add times, and per-pattern scopes that return the before and after values of a superseded fact.

#### Scenario: Per-pattern before and after
- **WHEN** the superseded-fact fixture runs SPARQL `SELECT ?before ?after WHERE { SERVICE <urn:tiramemsu:tm:asOf/3> { v:alice v:worksAt ?before } v:alice v:worksAt ?after . FILTER (?before != ?after) }` and the Cypher `CALL { USE AS OF 3 … }` equivalent
- **THEN** both return `(v:acme, v:globex)`

#### Scenario: History with transaction numbers
- **WHEN** SPARQL `SELECT ?c ?t FROM <urn:tiramemsu:tm:history> WHERE { v:alice v:worksAt ?c ~ ?r . ?r tm:txAdded ?t }` and Cypher `USE HISTORY MATCH (a)-[r:worksAt]->(c) WHERE elementId(a) = 'urn:tiramemsu:v:alice' RETURN c, r.txAdded AS t` run
- **THEN** both return the same `(c, t)` multiset

### Requirement: Documented semantic divergences
The corpus SHALL include divergence pairs that pin the intended differences between the dialects. Each divergence pair SHALL assert the exact expected results on each side, not their equality. The pairs SHALL cover: relationship isomorphism versus homomorphism (and their convergence under `REPEATABLE ELEMENTS` or an explicit `FILTER(?r1 != ?r2)`); parallel edges as a bag of eids versus a set of triples (and their convergence in `set` mode); and multi-valued properties (Cypher list versus SPARQL rows).

#### Scenario: Isomorphism divergence and convergence
- **WHEN** on a single `knows` edge, SPARQL `SELECT * WHERE { ?x v:knows ?y . ?z v:knows ?y }` and Cypher `MATCH (x)-[:knows]->(y)<-[:knows]-(z) RETURN *` run
- **THEN** SPARQL returns 1 row and Cypher returns 0 rows
- **AND** with `MATCH REPEATABLE ELEMENTS` the Cypher side returns the same 1 row

#### Scenario: Parallel edges
- **WHEN** the fixture creates `(v:alice v:called v:bob)` twice and both dialects list `called` pairs
- **THEN** Cypher returns 2 rows and SPARQL returns 1 row
- **AND** the pair in `set` mode passes

### Requirement: Suite execution and reporting
The suite SHALL run as part of the workspace test command. Any mismatch SHALL fail the build. The failure report SHALL name the pair, the fixture, both query texts, and the normalised difference (rows only on the SPARQL side, rows only on the Cypher side). The suite SHALL be enabled only when both front ends are built. Until the SPARQL front end is complete, the pairs SHALL be compiled and checked for Cypher-side validity only, and the remaining pairs SHALL be reported as pending, not passed.

#### Scenario: Mismatch report
- **WHEN** a pair's results differ by one row
- **THEN** the test fails, and the output contains the pair name, both queries, and the extra row on its side

#### Scenario: SPARQL front end missing
- **WHEN** the suite runs in a build without the SPARQL front end
- **THEN** every Cypher query in the corpus still compiles successfully, and the report lists the pairs as pending
