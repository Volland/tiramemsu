# cypher-temporal-clauses Specification

## Purpose
Defines the Cypher time syntax. `USE AS OF`, `USE VALID AT` and `USE HISTORY` select the transaction-time and valid-time view for a whole query or for a single `CALL { … }` scope. Statement time metadata is exposed as the properties `txAdded`, `txRetracted`, `validFrom` and `validTo`.

## Requirements

### Requirement: Default time view
A query without a `USE` clause SHALL evaluate every pattern, property lookup and label test under the view of the handle it runs on. The now view SHALL mean transaction time `Now` with valid time unfiltered. Valid-time filtering SHALL never be applied implicitly.

#### Scenario: Past episodes visible by default
- **WHEN** `(v:alice v:worksAt v:acme)` is live with valid interval [2020-01-01, 2022-01-01) and `MATCH (a)-[:worksAt]->(c) RETURN c` runs on the now view
- **THEN** `v:acme` is returned

### Requirement: USE AS OF a transaction number
`USE AS OF <t>`, where `<t>` is an integer literal or an integer parameter, SHALL evaluate the query as of transaction `t`: statements with `t_add ≤ t` and (`t_ret` null or `t_ret > t`). A `t` smaller than 1 SHALL give the empty view. A `t` larger than the last committed transaction SHALL give the latest committed state. A non-integer value MUST fail: with a `Parse` error for a literal, and with an `Eval` error for a parameter.

#### Scenario: Superseded value as of an earlier tx
- **WHEN** `(v:alice v:worksAt v:acme)` was asserted in tx 3 and superseded in tx 7 by `(v:alice v:worksAt v:globex)`, and `` USE AS OF 5 MATCH (a {`@id`: 'v:alice'})-[:worksAt]->(c) RETURN c `` runs
- **THEN** `c` is `v:acme`
- **AND** the same query with `USE AS OF 7` returns `v:globex`

#### Scenario: Before the first transaction
- **WHEN** `USE AS OF 0 MATCH (n) RETURN count(n) AS c` runs
- **THEN** `c` is 0

#### Scenario: Parameterised tx
- **WHEN** `USE AS OF $t MATCH (n:Person) RETURN count(n)` runs with `t = 5`
- **THEN** the result equals that of `USE AS OF 5 …`

#### Scenario: Float tx rejected
- **WHEN** `USE AS OF 5.5 MATCH (n) RETURN n` is compiled
- **THEN** it fails with a `Parse` error spanning `5.5`

### Requirement: USE AS OF a wall-clock instant
`USE AS OF <datetime>`, where the argument is a `datetime(…)` expression or a DateTime parameter, SHALL resolve to the largest transaction `t` whose commit instant is less than or equal to the given instant (the argument's timezone offset only locates the instant), and then behave as `USE AS OF t`. An instant earlier than the first transaction SHALL give the empty view.

#### Scenario: Instant resolves to a transaction
- **WHEN** tx 3 committed at 2026-09-01T10:00:00Z and tx 4 at 2026-09-01T12:00:00Z, and `USE AS OF datetime('2026-09-01T11:00:00Z') MATCH …` runs
- **THEN** the result equals that of `USE AS OF 3 MATCH …`

#### Scenario: Offset of the instant is honoured
- **WHEN** the same store is queried with `USE AS OF datetime('2026-09-01T13:00:00+02:00') MATCH …`
- **THEN** the result equals that of `USE AS OF 3 MATCH …`, because only the instant (11:00Z) is used

#### Scenario: Instant before any transaction
- **WHEN** `USE AS OF datetime('1999-01-01T00:00:00Z') MATCH (n) RETURN count(n) AS c` runs
- **THEN** `c` is 0

### Requirement: USE VALID AT
`USE VALID AT <instant>`, where the argument is a `date(…)` or `datetime(…)` expression or a Date or DateTime parameter, SHALL keep only statements whose valid interval `[v_from, v_to)` contains the instant, with a null bound meaning unbounded. A date SHALL denote 00:00:00 UTC of that day. Any other argument type MUST fail.

#### Scenario: Half-open interval
- **WHEN** `(v:alice v:worksAt v:acme)` has valid interval [2025-01-01, 2026-03-01) and the query `USE VALID AT date('2026-03-01') MATCH (a)-[:worksAt]->(c) RETURN c` runs
- **THEN** zero rows are returned
- **AND** the same query with `date('2026-02-28')` returns `v:acme`

#### Scenario: Unbounded statement always valid
- **WHEN** a statement has no valid interval and any `USE VALID AT` query that matches it runs
- **THEN** the statement is included

#### Scenario: Integer argument rejected
- **WHEN** `USE VALID AT 1700000000000 MATCH (n) RETURN n` is compiled
- **THEN** it fails with a `Parse` error

### Requirement: USE HISTORY
`USE HISTORY` SHALL evaluate the query over every statement ever asserted, live or retracted, with each eid appearing once. Property lookups under `HISTORY` SHALL consider all historical values.

#### Scenario: All versions of a relationship
- **WHEN** alice worked at acme (asserted tx 3, retracted tx 7) and works at globex (asserted tx 7), and `` USE HISTORY MATCH (a {`@id`: 'v:alice'})-[r:worksAt]->(c) RETURN c, r.txAdded AS added, r.txRetracted AS gone ORDER BY added `` runs
- **THEN** the rows are `(v:acme, 3, 7)` and `(v:globex, 7, null)`

### Requirement: Combined time selectors
A single `USE` clause SHALL accept at most one transaction-time selector (`AS OF …` or `HISTORY`), optionally followed by `VALID AT …`. A clause with only `VALID AT` SHALL leave the inherited transaction-time selector unchanged, and a clause with only a transaction-time selector SHALL leave the inherited valid-time selector unchanged. Two transaction-time selectors in one clause MUST fail with a `Parse` error.

#### Scenario: AS OF with VALID AT
- **WHEN** `USE AS OF 5 VALID AT date('2021-06-01') MATCH (a)-[:worksAt]->(c) RETURN c` runs
- **THEN** only relationships that were believed at tx 5 and valid on 2021-06-01 are returned

#### Scenario: Conflicting selectors
- **WHEN** `USE AS OF 5 HISTORY MATCH (n) RETURN n` is compiled
- **THEN** it fails with a `Parse` error

### Requirement: USE clause placement
A time `USE` clause SHALL be accepted only as the first clause of a query, as the first clause of each `UNION` branch, or as the first clause inside a `CALL { … }` body (after an importing `WITH`, if there is one). A `USE` anywhere else MUST fail with a `Parse` error spanning it. `USE <graph name>` (a graph reference instead of a time selector) MUST fail with `Unsupported`, because a database file holds one graph.

#### Scenario: USE in the middle of a query
- **WHEN** `MATCH (n) USE AS OF 3 RETURN n` is compiled
- **THEN** it fails with a `Parse` error spanning `USE AS OF 3`

#### Scenario: Graph name rejected
- **WHEN** `USE memory MATCH (n) RETURN n` is compiled
- **THEN** it fails with `Unsupported`

### Requirement: Per-pattern time scopes through CALL subqueries
A `USE` clause inside a `CALL { … }` body SHALL set the view of every pattern, property lookup and label test inside that body, overriding the enclosing view selector by selector. Patterns outside the body SHALL keep the enclosing view. A nested `CALL` without its own `USE` SHALL inherit its enclosing body's view. Imported variables SHALL keep their identity across scopes. Properties of an imported variable that are read inside the body SHALL be read under the body's view.

#### Scenario: What changed since tx 150
- **WHEN** alice worked at acme as of tx 150 and at globex now, and `` MATCH (a {`@id`: 'v:alice'})-[:worksAt]->(after) CALL { WITH a USE AS OF 150 MATCH (a)-[:worksAt]->(before) RETURN before } WITH before, after WHERE before <> after RETURN before, after `` runs
- **THEN** one row `(v:acme, v:globex)` is returned

#### Scenario: Property read under the scope's view
- **WHEN** alice's name was "Alicia" as of tx 5 and is "Alice" now, and `` MATCH (a {`@id`: 'v:alice'}) CALL { WITH a USE AS OF 5 RETURN a.name AS old } RETURN a.name AS now, old `` runs
- **THEN** the row is `now = "Alice"`, `old = "Alicia"`

#### Scenario: Nested scope inherits
- **WHEN** `CALL { USE AS OF 5 CALL { MATCH (n:Person) RETURN count(n) AS c } RETURN c } RETURN c` runs
- **THEN** `c` equals the number of `Person` nodes as of tx 5

### Requirement: In-query clauses override the handle view
When a query with a top-level `USE` clause runs on a handle that already carries a time selection, each selector given in the clause SHALL replace the handle's corresponding selector, and each selector not given SHALL be kept from the handle.

#### Scenario: Query AS OF overrides the handle AS OF
- **WHEN** `USE AS OF 7 MATCH (n:Person) RETURN count(n)` runs on the view as of tx 3
- **THEN** the result equals running it on the now view with `USE AS OF 7`

#### Scenario: Handle valid time kept
- **WHEN** `USE AS OF 7 MATCH (a)-[:worksAt]->(c) RETURN c` runs on the now view restricted to valid time 2021-06-01
- **THEN** the result equals `USE AS OF 7 VALID AT date('2021-06-01') …` on the now view

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
