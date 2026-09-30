## MODIFIED Requirements

### Requirement: Query results

`sparql` SHALL return `{"kind": "select", "vars", "rows"}` with each row an object of the bound variables, `{"kind": "ask", "value"}`, `{"kind": "graph", "triples"}` or `{"kind": "update", "report"}`. `sparql` SHALL accept an optional boolean `provenance`; when it is `true`, a select result SHALL also have `"provenance"`, an array parallel to `"rows"` whose entries list the row's statements as `{"stmt": n}` in ascending order, and the other forms SHALL fail with code `Unsupported`. `cypher` SHALL return `{"columns", "rows"}`. `triples` SHALL return one object per statement with its eid, its three terms, `tAdd`, `tRet`, `validFrom`, `validTo` and `retKind`. A `triples` pattern that names a term that is not stored SHALL return no rows and SHALL NOT fail.

#### Scenario: Cypher rows
- **WHEN** a Cypher write creates two people linked by KNOWS and a read returns both names
- **THEN** the result has columns `["a", "b"]` and one row `["Alice", "Bob"]`

#### Scenario: Unstored term
- **WHEN** `triples` is called with a subject that was never asserted
- **THEN** it returns an empty list

#### Scenario: SPARQL rows with provenance
- **WHEN** `sparql` runs `SELECT ?o WHERE { v:a v:p ?o }` with `"provenance": true` on a store where `(v:a v:p v:b)` is statement 1
- **THEN** the result has `"rows": [{"o": …}]` and `"provenance": [[{"stmt": 1}]]`, and without the argument it has no `"provenance"` member
