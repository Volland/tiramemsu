# Valid-time writes from SPARQL, statement ids and transactions in Cypher

## Status

Proposed and implemented 2026-10-14.

## Why

Valid time is half of the data model, but SPARQL can only read it. `tm:validFrom` and `tm:validTo` work in a `WHERE`, while every update that names them fails with `ReservedNamespace`, and the `sparql-update` capability says so on purpose ("Valid time SHALL NOT be settable through SPARQL Update in v1"). A SPARQL-only client, such as an endpoint user or an LLM that writes SPARQL, cannot record "Alice worked at Acme from 2020" or close an interval ("she left in March") without leaving the language. Cypher can do both (`CREATE … {validFrom: …}`, `SET r.validTo`).

The other direction has the opposite gap. From a triple, SPARQL reaches its statement id (`~ ?e`), its transaction (`?e tm:txAdded ?tx`) and that transaction's metadata (`?tx sys:author ?a`). Cypher stops at an Integer: `r.txAdded` is a number, `id(r)` is the raw ObjectId rather than the statement id, and a transaction cannot be reached as an entity, so "who wrote this fact, and why" has no Cypher answer. A transaction IRI in a Cypher map even fails with a type error.

## What Changes

- **SPARQL Update writes valid time.** A `tm:validFrom` or `tm:validTo` triple in an insert template or `INSERT DATA`, whose subject denotes a statement, sets that bound of the statement's valid time instead of storing a triple:
  - on a statement the same request asserts through a reifier or an annotation (`v:a v:p v:b {| tm:validFrom "2020-01-01"^^xsd:date |}`), the bounds are part of the assert, so its idempotency check uses that interval;
  - on an existing statement (a bound `?e`, a statement IRI, or a reifier bound to a stored eid) the statement is superseded with the bounds patched, as Cypher `SET r.validFrom` does, with one supersede per statement and operation;
  - in a delete template (`DELETE { ?e tm:validTo ?t }`), the bound is cleared to unbounded when the deleted value is the current one. A delete and an insert of the same bound in one operation set it.
  - Values are `xsd:dateTime`, `xsd:date` (midnight UTC) or `xsd:integer` epoch milliseconds.
- **Transaction time stays read-only** in SPARQL, with a clear error. `tm:txAdded`, `tm:txRetracted`, `tm:addedAt`, `tm:retractedAt` and `tm:retractKind` in an update template fail with `Unsupported` before the transaction opens.
- **Cypher reads statement ids and transactions.**
  - `tiramemsu.statementId(r)` is the statement id of a relationship or a statement node, as an Integer (the `n` of `urn:tiramemsu:stmt:n`).
  - `tiramemsu.transaction(r)` is the transaction that added it, as a node. `tiramemsu.retraction(r)` is the transaction that retracted it, or `null` while it is live.
  - A transaction node has the label `Transaction`, the element id `urn:tiramemsu:tx:<t>`, and its metadata (`sys:author`, `sys:source`, `sys:reason`, user predicates) as properties. It also has the read-only properties `number` (Integer) and `instant` (DateTime).
  - A transaction IRI written in a Cypher pattern (`` {`@id`: 'urn:tiramemsu:tx:3'} ``) binds that transaction node.
- **SPARQL needs no new syntax for ids and transactions.** The spec records the existing forms (`~ ?e`, `tm:txAdded`, `?tx sys:…`, `?e tm:txAdded <tx>`) as requirements with scenarios, so both dialects are tested to agree.

## Capabilities

### Modified Capabilities

- `sparql-update`: valid-time writes, and transaction time refused at plan time. This replaces "Virtual predicates are read-only".
- `cypher-temporal-clauses`: the functions `tiramemsu.statementId`, `tiramemsu.transaction` and `tiramemsu.retraction`, and transaction nodes.
- `cypher-dual-view`: statement id and transaction parity between SPARQL and Cypher.

## Impact

- `tm-sparql`: update planning (template check) and the update runner (valid-time interception).
- `tm-cypher`: the function registry and executor, transaction nodes in property access and labels, and the binding of a transaction IRI.
- Tests in `crates/tiramemsu/tests`, `lat.md/query.md` and `lat.md/tests.md`.
- The CTEG article, which can now write valid time in SPARQL.

No storage format change. One behaviour change: a transaction term in Cypher (a transaction IRI in a map, or the object of `sys:confirmedBy`) is now a node rather than an Integer. `r.txAdded` and `r.txRetracted` stay Integers.

## Non-goals

- No SPARQL write of transaction time.
- No `MATCH (t:Transaction)` label scan over all transactions; transaction nodes are reached from statements or by IRI.
- No change to `id()`, which stays the raw ObjectId as documented.
- No `MERGE` with valid-time properties.
