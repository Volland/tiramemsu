## MODIFIED Requirements

### Requirement: Unsupported combinations fail before execution

Cypher SHALL keep its single graph: a Cypher `USE` clause naming a graph SHALL fail with `Unsupported { feature: "USE GRAPH" }`. A property path inside a `GRAPH` block or under a `FROM <g>` default graph is not an unsupported combination: it SHALL be evaluated with the graph set of its block, as the `path-lowering` capability specifies ("SPARQL paths inside named graphs").

#### Scenario: Path inside GRAPH
- **WHEN** `(v:a v:knows v:b)` is in `<g1>`, `(v:b v:knows v:c)` is in no graph, and `SELECT ?x WHERE { GRAPH <g1> { v:a v:knows+ ?x } }` is submitted
- **THEN** the only solution is `?x = v:b`, and no error is raised

#### Scenario: Cypher USE GRAPH
- **WHEN** `USE GRAPH g1 MATCH (n) RETURN n` is submitted through Cypher
- **THEN** the request fails with `Unsupported { feature: "USE GRAPH" }`
