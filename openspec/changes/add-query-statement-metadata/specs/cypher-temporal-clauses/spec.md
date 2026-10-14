## ADDED Requirements

### Requirement: Statement id and transaction functions

For a relationship, or a statement in node form:
- `tiramemsu.statementId(x)` SHALL return the statement id as an Integer: the `n` of its element id `urn:tiramemsu:stmt:<n>`.
- `tiramemsu.transaction(x)` SHALL return the transaction that added the statement, as a transaction node.
- `tiramemsu.retraction(x)` SHALL return the transaction that retracted it, as a transaction node, or `null` when the statement has no retraction (when `r.txRetracted` is `null`).

All three SHALL return `null` for `null`, and SHALL fail with `Eval` for any other value.

#### Scenario: Statement id
- **WHEN** `v:alice v:worksAt v:acme` is statement 3 and `MATCH ()-[r:worksAt]->() RETURN tiramemsu.statementId(r) AS id, elementId(r) AS eid` runs
- **THEN** the row is `3, "urn:tiramemsu:stmt:3"`

#### Scenario: Retraction of a live statement
- **WHEN** `MATCH ()-[r:worksAt]->() RETURN tiramemsu.retraction(r)` runs on a live statement
- **THEN** the value is `null`

#### Scenario: Why a fact was retracted
- **WHEN** the statement was retracted in a transaction with `sys:reason "left the company"`, and ``USE HISTORY MATCH ()-[r:worksAt]->() RETURN tiramemsu.retraction(r).`sys:reason` AS why`` runs
- **THEN** the value is `"left the company"`

### Requirement: Transaction nodes

A transaction SHALL be a node with:
- the label `Transaction`;
- the element id `urn:tiramemsu:tx:<t>`;
- one property per triple with the transaction as subject, named as other properties are (for example `` t.`sys:author` ``);
- the read-only properties `number` (Integer `t`) and `instant` (the commit instant, a DateTime), which `keys()` SHALL NOT list.

A transaction IRI in a node map (`` {`@id`: 'urn:tiramemsu:tx:3'} ``) SHALL bind that transaction node. `SET` or `REMOVE` on a transaction node SHALL fail with `Unsupported`. `r.txAdded` and `r.txRetracted` SHALL remain Integers.

#### Scenario: Who wrote a fact, in Cypher
- **WHEN** a transaction with `sys:author "agent-7"` asserted the statement, and ``MATCH ()-[r:worksAt]->() WITH tiramemsu.transaction(r) AS t RETURN t.number, t.`sys:author`, labels(t), elementId(t)`` runs
- **THEN** the row is `r.txAdded`, `"agent-7"`, `["Transaction"]` and `"urn:tiramemsu:tx:<r.txAdded>"`

#### Scenario: Transaction instant
- **WHEN** `MATCH ()-[r:worksAt]->() RETURN tiramemsu.transaction(r).instant = r.addedAt AS same` runs
- **THEN** `same` is true

#### Scenario: Transaction by IRI
- **WHEN** ``MATCH (t {`@id`: 'urn:tiramemsu:tx:1'}) RETURN t.`sys:author` `` runs
- **THEN** it returns the author of transaction 1

#### Scenario: Transactions are read-only
- **WHEN** ``MATCH ()-[r:worksAt]->() WITH tiramemsu.transaction(r) AS t SET t.`sys:author` = 'me'`` runs
- **THEN** it fails with `Unsupported`, and nothing is written
