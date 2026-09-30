## MODIFIED Requirements

### Requirement: SPARQL JSON results

`SELECT` and `ASK` results SHALL be serialisable as SPARQL 1.1 Query Results JSON. For `SELECT`, `head.vars` SHALL list the projected variables in projection order, and each row SHALL be an object in `results.bindings` that omits unbound variables. IRIs and skolem IRIs SHALL use `"type": "uri"`. Literals SHALL use `"type": "literal"`, with `"xml:lang"` for language strings and `"datatype"` for every typed literal except plain strings. For `ASK`, the document SHALL be `{"head": {}, "boolean": <answer>}`. Row order SHALL be preserved when the query has `ORDER BY`. A `SELECT` run with provenance (capability `query-provenance`) SHALL add the non-standard top-level member `"provenance"` between `"head"` and `"results"`; without provenance the document SHALL contain only the standard members.

#### Scenario: SELECT JSON document
- **WHEN** `SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age } }` returns one row with `p = v:bob` and `age` unbound, and the result is serialised
- **THEN** the output is `{"head":{"vars":["p","age"]},"results":{"bindings":[{"p":{"type":"uri","value":"urn:tiramemsu:v:bob"}}]}}`, ignoring whitespace

#### Scenario: Typed literal JSON
- **WHEN** a row binds `a` to the integer 41 and is serialised
- **THEN** the binding is `{"type":"literal","value":"41","datatype":"http://www.w3.org/2001/XMLSchema#integer"}`

#### Scenario: ASK JSON document
- **WHEN** an `ASK` query answers true and is serialised
- **THEN** the output is `{"head":{},"boolean":true}`, ignoring whitespace

#### Scenario: No provenance member by default
- **WHEN** the same `SELECT` runs with `View::sparql` and with `View::sparql_with` and provenance off
- **THEN** both documents are byte-identical and have no `"provenance"` member
