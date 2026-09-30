## ADDED Requirements

### Requirement: Out-of-subset constructs fail before execution

Constructs outside the v1 subset SHALL be rejected after parsing and before any SQL is executed, with `Unsupported { feature }` naming the construct. This SHALL cover at least: `DESCRIBE` (`"DESCRIBE"`), `SERVICE` with a variable or with an IRI that is not a time IRI, that is federation (`"SERVICE"`), custom aggregates, the functions listed as unsupported, and the property paths of the interim path requirement. Property paths inside `GRAPH` or under a `FROM <g>` default graph SHALL be rejected with `"named graph path"`. A rejected query SHALL return no partial results. `SERVICE` with a `tm:` time IRI is not federation: it is the per-group time scope of the temporal dataset capability. `GRAPH` with a `tm:` IRI is a `Parse` error that names `SERVICE`, as that capability specifies. `GRAPH`, `GRAPH ?g`, `FROM` and `FROM NAMED` with other IRIs are named graphs, specified by the `named-graphs` capability.

#### Scenario: Federated SERVICE is rejected
- **WHEN** `SELECT * WHERE { SERVICE <http://dbpedia.org/sparql> { ?s ?p ?o } }` is submitted
- **THEN** the request fails with `Unsupported { feature: "SERVICE" }`

#### Scenario: GRAPH variable is evaluated
- **WHEN** `SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }` is submitted on a store where one statement is a member of `<g1>`
- **THEN** one row with `g = <g1>` is returned and no error is raised

#### Scenario: Path inside GRAPH is rejected
- **WHEN** `SELECT ?x WHERE { GRAPH <g1> { v:a v:knows+ ?x } }` is submitted
- **THEN** the request fails with `Unsupported { feature: "named graph path" }`

## REMOVED Requirements

### Requirement: Unsupported features fail before execution
**Reason**: Renamed and reworded: `GRAPH ?g`, `FROM`, `FROM NAMED` and graph IRIs are no longer unsupported, so its "GRAPH variable is rejected" scenario no longer holds. The replacement keeps the remaining rejections and adds the graph-path rejection.
**Migration**: Queries that were rejected with `Unsupported { feature: "named graph" }` or `"GRAPH variable"` now run, as the `named-graphs` capability specifies.
