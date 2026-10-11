## ADDED Requirements

### Requirement: Origin registry and fleet number
In files of the merge storage format, the table `origin(n, uuid, label, registered_t)` SHALL record every known origin, including the file's own UUID, and `meta.origin` SHALL hold the file's fleet number, 0 until set. `Db::set_origin(n)` SHALL accept 1 ≤ n ≤ 4 095, SHALL succeed only while `meta.origin` is 0 or equal to n and no change set has been exported, and SHALL otherwise fail with `Unsupported`. Merging a change set from an unknown UUID SHALL register it under the change set's origin number; a number already registered to another UUID SHALL fail the merge with `OriginConflict` and leave no trace.

#### Scenario: Joining a fleet
- **WHEN** a migrated file with `meta.origin = 0` calls `set_origin(2)`
- **THEN** `meta.origin` is 2 and the file's own registry row has number 2

#### Scenario: Duplicate fleet number
- **WHEN** files A and B both use fleet number 2, and A merges a change set exported by B
- **THEN** the merge fails with `OriginConflict` naming number 2 and both UUIDs, and nothing is written

### Requirement: Canonical ids and foreign identity
Ids allocated by a file SHALL keep origin 0 on disk. Outside the file, an id with origin 0 SHALL have the canonical form `(meta.origin, counter)`, and exporting requires `meta.origin` ≠ 0 (else `OriginUnset`). On merge, an id whose origin equals the target's `meta.origin` SHALL map to origin 0, and every other origin SHALL be stored unchanged, so a statement, node or blank node SHALL have one canonical id in every file that holds it. A non-zero origin that is not registered, or equal to the file's own number, SHALL be rejected on input.

#### Scenario: Statement keeps its id across files
- **WHEN** file A (origin 1) asserts statement 5 and file B (origin 2) merges A's change set, then file C (origin 3) merges B's change set
- **THEN** B and C both hold the statement with origin 1 and counter 5, and A still holds it as local statement 5

#### Scenario: Own statements come home
- **WHEN** B's change set contains statement `(1, 5)` and A (origin 1) merges it
- **THEN** A recognises it as its local statement 5 and inserts nothing

### Requirement: Skolem IRIs for foreign ids
A foreign id SHALL render and parse as `urn:tiramemsu:stmt:<origin>.<n>`, `urn:tiramemsu:node:<origin>.<n>`, `urn:tiramemsu:bnode:<origin>.<n>` or `urn:tiramemsu:tx:<origin>.<n>`. A local id SHALL keep the form without an origin, and `<self>.<n>` with the file's own number SHALL parse to the same local id. The decimal form with the origin folded into the number SHALL remain rejected.

#### Scenario: Cited IRIs keep working
- **WHEN** an agent stored `urn:tiramemsu:stmt:5` before the file joined a fleet as origin 1
- **THEN** after joining, `urn:tiramemsu:stmt:5` and `urn:tiramemsu:stmt:1.5` both resolve to that statement in SPARQL and Cypher

#### Scenario: Foreign statement in results
- **WHEN** a statement merged from origin 2 with counter 7 is returned by a SPARQL query
- **THEN** it is rendered as `urn:tiramemsu:stmt:2.7`

### Requirement: Merge is union with earliest retraction
`Tx::merge(&ChangeSet)` SHALL insert every statement whose canonical id is unknown, with its content and valid time, resolving references within the change set before insertion so that reference cycles merge; SHALL leave a known statement with equal content unchanged; SHALL fail with `MergeConflict` and write nothing when a known id arrives with different content; SHALL retract, with the reported kind, every locally live statement the change set reports retracted, cascading local dependents with kind `cascade`; and SHALL leave a locally retracted statement retracted. A merge SHALL NOT apply `sys:one` replacement or `sys:unique` checks against statements of other origins. Merged statements SHALL get the merge transaction as `t_add`, and their original origin transaction and instant SHALL be recorded in `merged_from`. Transaction ids of other origins SHALL occur only as values.

#### Scenario: Order does not matter
- **WHEN** three files exchange change sets in two different orders until each has merged the others
- **THEN** every file holds the same set of (canonical id, content, retracted) triples in both orders

#### Scenario: Remote retraction cascades local layers
- **WHEN** A holds B's statement `(2, 5)` with a local layer `(stmt:2.5 v:note "x")`, and B's change set reports `(2, 5)` retracted with kind `explicit`
- **THEN** after the merge `(2, 5)` is retracted with kind `explicit` and the local note with kind `cascade`, both in the merge transaction

#### Scenario: Content mismatch
- **WHEN** a change set contains id `(2, 5)` with a different object than the local copy
- **THEN** the merge fails with `MergeConflict` naming the id and nothing is written

#### Scenario: Origin time kept
- **WHEN** B's statement `(2, 5)` was asserted in B's transaction 41 at instant T and A merges it in its transaction 90
- **THEN** in A its `t_add` is 90 and `merged_from` records origin 2, origin transaction 41 and instant T

### Requirement: Incremental change sets and preview
`Db::export_changes(since_t)` SHALL return a change set with the source's UUID and origin, the local range `(since_t, last_t]`, and, references first, every statement of any origin asserted in that range with its canonical id, content, valid time and origin time, and every retraction in that range with its kind; its JSON form SHALL be `tiramemsu-changeset/1`. `sync_peer` SHALL record per peer UUID the last received and last sent local transaction. `Db::preview_merge(&ChangeSet)` SHALL run the merge as a dry run and report proposed inserts, retractions, local cascades, schema failures, origins that would be registered and burned ids, committing nothing; applying SHALL be a separate ordinary transaction that revalidates.

#### Scenario: Only new changes are sent
- **WHEN** A exported to B up to its transaction 50 and then commits transactions 51 and 52
- **THEN** `export_changes(50)` contains exactly the events of transactions 51 and 52

#### Scenario: Preview matches apply
- **WHEN** `preview_merge(cs)` is called and then `merge(cs)` is applied with no write in between
- **THEN** the merge inserts and retracts exactly what the preview reported, and the preview committed no row

### Requirement: Cross-origin cardinality as read policies
Local writes SHALL enforce `sys:one` and `sys:unique` against the live statements of all origins. Violations created by merges SHALL be reported by `View::conflicts`, which SHALL accept a `cross_origin` filter and SHALL include each statement's origin in its evidence. A view option `origin_precedence` (an ordered list of origins) SHALL, for `sys:one` predicates, hide every value whose origin ranks below the origin of another value of the same subject with overlapping valid time, through the same visibility predicates as every other view filter.

#### Scenario: Conflict after merge
- **WHEN** `v:age` is `sys:one`, A holds `(v:alice v:age 30)` and merges B's `(v:alice v:age 31)` with overlapping valid time
- **THEN** both statements stay live, and `View::conflicts` with `cross_origin: true` lists them with origins A and B

#### Scenario: Read-time precedence
- **WHEN** the same data is read through a view with `origin_precedence = [B, A]`
- **THEN** `v:alice v:age ?a` returns only 31, in SPARQL, Cypher and the Rust API

#### Scenario: Local write resolves and syncs
- **WHEN** A then asserts `(v:alice v:age 32)` locally and B later merges A's change set
- **THEN** both earlier values are retracted in A by cardinality replacement, and in B after the merge
