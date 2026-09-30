## MODIFIED Requirements

### Requirement: Statement time properties
For a relationship, or a statement in node form, the property names `txAdded`, `txRetracted`, `addedAt`, `retractedAt`, `validFrom` and `validTo` SHALL return the statement's `t_add` (Integer), its `t_ret` (Integer, or `null` while live), the commit instant of `t_add` (DateTime), the commit instant of `t_ret` (DateTime, or `null` while live), its `v_from` (DateTime, or `null` when unbounded) and its `v_to` (DateTime, or `null` when unbounded). On statements these six names SHALL denote the time metadata even if a property statement with the same resolved name exists; such a stored property SHALL remain reachable through its CURIE or full IRI. The CURIE forms `` r.`tm:txAdded` ``, `` r.`tm:txRetracted` ``, `` r.`tm:addedAt` ``, `` r.`tm:retractedAt` ``, `` r.`tm:validFrom` `` and `` r.`tm:validTo` `` SHALL be equivalent to these names, and `` r.`tm:retractKind` `` SHALL return `"explicit"`, `"cascade"`, `"supersede"`, `"cardinality"` or `null`. None of these SHALL appear in `keys()` or `properties()`. On ordinary nodes the six names SHALL be ordinary property keys. A relationship property map in `MATCH` SHALL test these names against the metadata in the same way.

#### Scenario: Time metadata of a live relationship
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` was asserted in tx 3 with valid interval [2025-01-01, open), and `MATCH (a)-[r:worksAt]->(c) RETURN r.txAdded, r.txRetracted, r.validFrom, r.validTo` runs
- **THEN** the row is `3, null, 2025-01-01T00:00:00.000Z, null`

#### Scenario: Commit instants of a relationship
- **WHEN** e1 was added in a transaction committed at 2026-03-10T00:00:00Z and `MATCH (a)-[r:worksAt]->(c) RETURN r.addedAt, r.retractedAt` runs
- **THEN** the row is `2026-03-10T00:00:00.000Z, null`

#### Scenario: Retraction instant in history
- **WHEN** e1 was retracted in a transaction committed at 2026-03-12T00:00:00Z and `USE HISTORY MATCH ()-[r:worksAt]->() RETURN r.retractedAt` runs
- **THEN** the value is `2026-03-12T00:00:00.000Z`

#### Scenario: Retract kind in history
- **WHEN** e2 was retracted by cascade and `` USE HISTORY MATCH ()-[r]->() WHERE r.txRetracted IS NOT NULL RETURN r.`tm:retractKind` AS k `` runs
- **THEN** the row for e2 has `k = "cascade"`

#### Scenario: Same metadata on the node form
- **WHEN** `MATCH (:Belief)-[:SUPPORTED_BY]->(x) RETURN x.txAdded, x.addedAt` runs, where x is e1
- **THEN** the values equal `r.txAdded` and `r.addedAt` for e1

#### Scenario: Time metadata not listed as keys
- **WHEN** `MATCH ()-[r:worksAt]->() RETURN keys(r)` runs for e1, which has the single property `v:confidence`
- **THEN** the value is `["confidence"]`

#### Scenario: Shadowed user property reachable by CURIE
- **WHEN** `(e1 v:txAdded "custom")` and `(e1 v:addedAt "custom")` are live and `` MATCH ()-[r:worksAt]->() RETURN r.txAdded, r.`v:txAdded`, r.`v:addedAt` `` runs
- **THEN** the row is `3, "custom", "custom"`, and `r.addedAt` is still the commit instant

#### Scenario: Ordinary node key
- **WHEN** `(v:alice v:validFrom "someday")` is live and `` MATCH (n {`@id`: 'v:alice'}) RETURN n.validFrom `` runs
- **THEN** the value is `"someday"`

#### Scenario: Learned late by more than N days
- **WHEN** e1 is valid from 2026-03-09 and e2 from 2026-01-01, both added at 2026-03-10, and `MATCH ()-[r]->() WHERE r.addedAt.epochMillis - r.validFrom.epochMillis > $days * 86400000 RETURN r` runs with `days = 30`
- **THEN** the result is e2 only

#### Scenario: Recorded after it stopped being true in Cypher
- **WHEN** e3 is valid [2025-01-01, 2025-06-01) and added at 2026-03-10, and `MATCH ()-[r]->() WHERE r.addedAt > r.validTo RETURN r` runs
- **THEN** the result is e3, and statements whose `validTo` is later or `null` are not returned

### Requirement: Time metadata is read-only except valid time
`SET` of `validFrom` or `validTo` on a statement SHALL supersede the statement with the corresponding bound patched; `null` means unbounded. After that, the variable SHALL refer to the new eid in the rest of the query. `SET` or `REMOVE` of `txAdded`, `txRetracted`, `addedAt` or `retractedAt` on a statement MUST fail with `Unsupported`. An empty or inverted interval MUST fail with `InvalidPatch`.

#### Scenario: Close an open interval
- **WHEN** e1 has valid interval [2025-01-01, open) and `MATCH ()-[r:worksAt]->() SET r.validTo = date('2026-03-01') RETURN r.validTo AS t, elementId(r) AS id` runs
- **THEN** e1 is retracted with kind `supersede`, and a new statement with interval [2025-01-01, 2026-03-01) is live
- **AND** `id` is the new statement's element id, and `t` is 2026-03-01T00:00:00.000Z

#### Scenario: txAdded is read-only
- **WHEN** `MATCH ()-[r:worksAt]->() SET r.txAdded = 1` runs
- **THEN** it fails with `Unsupported`

#### Scenario: addedAt is read-only
- **WHEN** `MATCH ()-[r:worksAt]->() SET r.addedAt = datetime()` or `MATCH ()-[r:worksAt]->() REMOVE r.retractedAt` runs
- **THEN** it fails with `Unsupported`, and nothing is written

#### Scenario: Inverted interval
- **WHEN** e1 starts 2025-01-01 and `SET r.validTo = date('2024-01-01')` runs on it
- **THEN** it fails with `InvalidPatch`, and nothing changes
