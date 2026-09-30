## MODIFIED Requirements

### Requirement: Time- and graph-scoped WHERE in updates

The `WHERE` pattern of an update SHALL accept the time IRIs of the temporal dataset capability: in `SERVICE` groups as per-group scopes, and in `USING` as the default for the `WHERE` pattern, with the same meaning that `FROM` has in queries. `USING` and `USING NAMED` with a non-`tm:` IRI, and `GRAPH` with a non-`tm:` IRI or a variable inside the `WHERE` pattern, SHALL name graphs as the `named-graphs` capability specifies. Inside the `WHERE` pattern, `GRAPH` with a time IRI and `SERVICE` with a non-time IRI or a variable SHALL fail exactly as they do in queries. Templates and data blocks SHALL always write to the current state, so a time IRI has no meaning there: a `GRAPH` block in `INSERT DATA`, `DELETE DATA` or a template SHALL name a graph as `named-graphs` specifies, and a time IRI as its name SHALL fail with a `Parse` error that names `SERVICE`. `WITH <g>` SHALL name the graph for template triples outside their own `GRAPH` block and SHALL scope the `WHERE` default graph to `g`. Property paths in the `WHERE` pattern inside `GRAPH` or under a `USING` graph default SHALL be evaluated with the graph set of their block, as the `path-lowering` capability specifies ("SPARQL paths inside named graphs").

#### Scenario: Restore a past value
- **WHEN** `(v:alice v:worksAt v:acme)` was live as of tx 150 and has since been retracted, and `INSERT { v:alice v:worksAt ?c } WHERE { SERVICE <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?c } }` is submitted
- **THEN** a new live statement `(v:alice v:worksAt v:acme)` with a new eid is asserted, and the old eid stays retracted

#### Scenario: Time IRI in GRAPH of a WHERE pattern is rejected
- **WHEN** `INSERT { v:alice v:worksAt ?c } WHERE { GRAPH <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?c } }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL whose message names `SERVICE`, and nothing is written

#### Scenario: Time IRI as the graph of INSERT DATA is rejected
- **WHEN** `INSERT DATA { GRAPH <urn:tiramemsu:tm:asOf/150> { v:a v:b v:c } }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL whose message names `SERVICE`, and nothing is written, because data is always written to the current state

#### Scenario: GRAPH in INSERT DATA names a graph
- **WHEN** `INSERT DATA { GRAPH <g1> { v:a v:b v:c } }` is submitted
- **THEN** `(v:a v:b v:c)` is live and a member of `g1`

#### Scenario: WITH names a graph
- **WHEN** `WITH <g1> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }` is submitted over a store where `(v:a v:b v:c)` is in `<g1>` and `(v:d v:e v:f)` is in no graph
- **THEN** the membership of `(v:a v:b v:c)` in `g1` is retracted, the statement stays live, and `(v:d v:e v:f)` is untouched

#### Scenario: Path inside GRAPH in a WHERE pattern
- **WHEN** `(v:a v:knows v:b)` and `(v:b v:knows v:c)` are in `<g1>`, and `INSERT { v:z v:reach ?x } WHERE { GRAPH <g1> { v:a v:knows+ ?x } }` is submitted
- **THEN** `(v:z v:reach v:b)` and `(v:z v:reach v:c)` are asserted
