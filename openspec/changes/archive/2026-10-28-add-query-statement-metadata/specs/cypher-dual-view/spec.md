## ADDED Requirements

### Requirement: Dialects agree on statement metadata

For the same statement, SPARQL and Cypher SHALL agree on the following:
- **Statement id.** The `n` of the SPARQL reifier binding `urn:tiramemsu:stmt:<n>` SHALL equal `tiramemsu.statementId(r)`.
- **Transaction.** The transaction bound by `tm:txAdded` SHALL equal the element id of `tiramemsu.transaction(r)`.
- **Valid time.** Bounds written in either language SHALL be read back the same by the other.

#### Scenario: Valid time written in SPARQL, read in Cypher
- **WHEN** `INSERT DATA { v:a v:p v:b {| tm:validFrom "2020-01-01"^^xsd:date |} }` runs and `MATCH ()-[r:p]->() RETURN r.validFrom` runs
- **THEN** the value is `2020-01-01T00:00:00.000Z`

#### Scenario: Valid time written in Cypher, read in SPARQL
- **WHEN** `MATCH ()-[r:p]->() SET r.validTo = date('2024-01-01')` runs and `SELECT ?t WHERE { v:a v:p v:b ~ ?e . ?e tm:validTo ?t }` runs
- **THEN** `?t` is `2024-01-01T00:00:00Z`

#### Scenario: Same id and transaction
- **WHEN** SPARQL `SELECT ?e ?tx WHERE { v:a v:p v:b ~ ?e . ?e tm:txAdded ?tx }` and Cypher `MATCH ()-[r:p]->() RETURN elementId(r), elementId(tiramemsu.transaction(r))` run on the same view
- **THEN** both return the same statement IRI and the same transaction IRI
