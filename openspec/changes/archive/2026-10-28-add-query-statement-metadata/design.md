# Design

## D1. Interception, like vector writes

SPARQL valid-time triples are not stored. They are engine operations, exactly like `tm:vectorEmbedding` (`lat.md/query#Vector Recall#Writes From Query Languages`). The planner classifies each template by predicate. The runner partitions an operation's instantiated triples into ordinary triples, vector writes and valid-time writes. Ordinary triples run first, so reifiers are bound and new annotations exist before a supersede moves them.

## D2. New statements: valid time is part of the assert

An annotation or reifier triple asserted by the same request is asserted with its bounds (`Tx::assert(s, p, o, Valid)`). Writing it first and then superseding would leave a spurious retracted statement in history for every insert. Because it is an assert, the documented idempotency applies: an overlapping live statement with the same content is returned unchanged, not widened. To change an existing interval, a client binds the eid in `DELETE/INSERT … WHERE`.

When the reified statement is already live with an overlapping interval, the request asserts nothing new and does not supersede. That matches `Tx::assert` and Cypher `MERGE` semantics, and the report lists the eid as existing.

## D3. Existing statements: one supersede per statement and operation

For a subject that denotes a stored statement, all valid-time inserts and deletes of the operation are merged into one `Patch { v_from, v_to }`:

- an insert sets `Some(Some(ms))`;
- a delete whose object equals the current bound sets `Some(None)`, and any other delete is a no-op, like deleting an absent triple;
- an insert wins over a delete of the same bound;
- two different inserted values for one bound of one statement are `InvalidQuery`.

A patch equal to the current interval writes nothing. Otherwise the result is `Tx::supersede(e, patch)`, whose errors (`InvalidPatch` for an empty or inverted interval, `NotLive`) surface unchanged. The supersede runs after the operation's ordinary triples, so annotations inserted on `?e` in the same operation move to the new statement.

The patches of a `DELETE/INSERT … WHERE` are collected over all solution rows before anything is applied. A statement bound by several rows is therefore superseded once.

## D4. Values

`xsd:dateTime` maps to its epoch milliseconds. `xsd:date` maps to midnight UTC. `xsd:integer` is epoch milliseconds. Anything else is `InvalidQuery`. The same mapping is used for the `timeRespecting` start (`lat.md/query#Temporal Path Syntax`).

## D5. Graph blocks

Valid time belongs to the statement, not to a membership. A valid-time triple inside a `GRAPH` block applies to the statement and writes no membership. This lets `INSERT DATA { GRAPH <g> { :a :p :b {| tm:validFrom … |} } }` work as written.

## D6. Cypher: namespaced functions, transaction nodes

The new functions live in the `tiramemsu.` namespace, as `tiramemsu.vector.search` does, so no openCypher name is taken. A transaction term becomes `Val::Node(Value::Tx(t))`:

- **Property access** reads triples with the transaction as subject, which is how transaction metadata is stored. `number` and `instant` are reserved names read from the `tx` row.
- **`labels(t)`** is `["Transaction"]`.
- **`elementId(t)`** is `urn:tiramemsu:tx:<t>`.
- **`id(t)`** is the raw ObjectId, as for every other node.

`tiramemsu.statementId` returns `Eid::n()`, the number in the statement's IRI and the `eid` the bindings return. It does not return the raw ObjectId that `id()` returns.

## D7. Transaction nodes are read-only

Transactions are engine-owned. Cypher has no way to write a transaction's metadata, which is `tx.meta` in the API, and `SET` on a transaction node fails with `Unsupported`.
