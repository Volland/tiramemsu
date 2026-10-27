# storage-trait Specification

## Purpose
A typed storage contract (`ReadStore`, `WriteStore`) with one transaction engine over it, so that a backend other than SQLite (redb, the in-memory reference store) runs the same semantics, proven equal to the SQLite engine by a differential oracle.

## Requirements

### Requirement: Typed storage operations
The storage contract SHALL be two Rust traits, `ReadStore` (one consistent snapshot) and `WriteStore` (one write transaction that also reads its own writes), whose operations take and return ObjectId integers and plain records. No operation SHALL take SQL text.

#### Scenario: Scan by bound positions
- **WHEN** a store holds live statements `(a p b)`, `(a p c)` and `(d p b)` and the engine scans the live scope with `s = a`, `p = p`
- **THEN** exactly the rows for `(a p b)` and `(a p c)` are returned, each with its eid, both clocks and its valid interval

#### Scenario: Scan in eid order
- **WHEN** the engine scans with `Order::Eid` and at least one position bound
- **THEN** the rows arrive in ascending eid order

#### Scenario: Live and all scopes
- **WHEN** a statement has been retracted
- **THEN** a `Live` scan omits it and an `All` scan returns it with its `t_ret` and `ret_kind`

### Requirement: Never forget by construction
`WriteStore` SHALL offer no operation that deletes a statement, a term or a transaction, or that changes a statement other than setting its retraction once. Inserting an existing eid, term id or transaction number SHALL fail.

#### Scenario: Second retraction is refused
- **WHEN** `retract(e)` is called on a statement that is already retracted
- **THEN** it returns false and the stored `t_ret` and `ret_kind` are unchanged

#### Scenario: Duplicate key is refused
- **WHEN** `insert_statement` is called with an eid that exists
- **THEN** it fails and the existing row is unchanged

### Requirement: One engine for every backend
`tm_store::Engine` SHALL implement every write operation of `tm_core::Tx` (assert, create, confirm, meta, upsert, retract, retract matching, supersede, graph membership, schema flags, volatile state, fact bundles, embeddings, text upkeep) and the reads `triples`, `graphs`, `graph_members`, `values`, `events_since`, `dependents` and conflict inspection, over any `WriteStore`/`ReadStore`, with the same errors and the same `TxReport`.

#### Scenario: Assert on the reference store
- **WHEN** a transaction asserts `(alice knows bob)` twice on a `MemStore`
- **THEN** the second assert returns `Existing` with the first eid and the report lists one asserted statement

#### Scenario: Cascade on the reference store
- **WHEN** a statement with two layers and a layer on a layer is retracted on a `MemStore`
- **THEN** all four statements are retracted, the root with `Explicit` and the rest with `Cascade`, in breadth-first eid order

### Requirement: Speculation without nested savepoints
Speculation and dry runs SHALL abort their write transaction and then persist the advanced id counters in a separate write transaction, so that a backend needs only begin, commit and abort.

#### Scenario: Dry run burns ids
- **WHEN** a dry run asserts one statement on a `MemStore` and a later transaction asserts another
- **THEN** the later statement's eid is greater than the eid reported by the dry run, and no statement or transaction from the dry run exists

### Requirement: Engine equivalence with SQLite
For every sequence of operations, the engine over `MemStore` SHALL produce the same reports or error kinds, the same statement rows, terms, transactions, counters, `pred_multi` rows and event log as `tm_core::Store` over SQLite.

#### Scenario: Random histories agree
- **WHEN** the differential property test runs generated operation sequences on both engines
- **THEN** every report, every error kind and the final stored state are identical

### Requirement: Snapshot reads
A `ReadStore` SHALL see one committed state for its whole lifetime: writes committed after it was opened SHALL NOT become visible through it.

#### Scenario: Reader ignores a later commit
- **WHEN** a reader is opened, then a transaction commits a new statement
- **THEN** a scan on the open reader does not return the new statement and a reader opened afterwards does
