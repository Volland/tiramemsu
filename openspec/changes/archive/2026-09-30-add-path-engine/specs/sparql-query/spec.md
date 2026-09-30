## MODIFIED Requirements

### Requirement: Unsupported features fail before execution

Constructs outside the v1 subset SHALL be rejected after parsing and before any SQL is executed, with `Unsupported { feature }` naming the construct. This SHALL cover at least: `DESCRIBE` (`"DESCRIBE"`), `SERVICE` with a variable or with an IRI that is not a time IRI, that is federation (`"SERVICE"`), `GRAPH` with a variable (`"GRAPH variable"`), `FROM`, `FROM NAMED` or `GRAPH` with an IRI outside the `tm:` namespace (`"named graph"`), custom aggregates, the functions listed as unsupported, and negated property sets (`"negated property sets"`). A rejected query SHALL return no partial results. `SERVICE` with a `tm:` time IRI is not federation: it is the per-group time scope of the temporal dataset capability. `GRAPH` with a `tm:` IRI is a `Parse` error that names `SERVICE`, as that capability specifies.

#### Scenario: Federated SERVICE is rejected
- **WHEN** `SELECT * WHERE { SERVICE <http://dbpedia.org/sparql> { ?s ?p ?o } }` is submitted
- **THEN** the request fails with `Unsupported { feature: "SERVICE" }`

#### Scenario: GRAPH variable is rejected
- **WHEN** `SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }` is submitted
- **THEN** the request fails with `Unsupported { feature: "GRAPH variable" }`

#### Scenario: Negated property set is rejected
- **WHEN** `SELECT ?x WHERE { v:alice !v:knows ?x }` is submitted
- **THEN** the request fails with `Unsupported { feature: "negated property sets" }`

## REMOVED Requirements

### Requirement: Property paths before the path engine
**Reason**: The path engine (add-path-engine, M3) evaluates every property path except negated property sets. The interim behaviour, `Unsupported { feature: "property path" }` for sequences, alternatives and closures, is replaced.
**Migration**: The `path-lowering` capability specifies SPARQL recursive property paths, non-recursive property paths (including the inverse `^iri`, which keeps its triple-pattern evaluation), endpoint binding, temporal scope and the unsupported negated sets.
