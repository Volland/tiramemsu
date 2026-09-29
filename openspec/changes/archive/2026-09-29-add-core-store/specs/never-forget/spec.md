## Purpose

Makes "nothing is ever deleted" a property of the database file itself: SQLite triggers reject every deletion and every content change of statements, terms and transactions, whether the write comes from the engine or from any other tool opening the file with plain SQLite.

## ADDED Requirements

### Requirement: Statements are never deleted
Any `DELETE` on the `triple` table SHALL be aborted by the database with the message `tiramemsu: triples are never deleted`, regardless of which connection issues it.

#### Scenario: Raw delete of one statement
- **WHEN** a plain SQLite connection, opened without the engine, runs `DELETE FROM triple WHERE eid = <e1>`
- **THEN** the statement fails with `tiramemsu: triples are never deleted`
- **AND** the row for `e1` is unchanged

#### Scenario: Delete of every statement
- **WHEN** a plain SQLite connection runs `DELETE FROM triple`
- **THEN** the statement fails and no row is removed

### Requirement: Rows are never replaced
An `INSERT` (including `INSERT OR REPLACE` and `REPLACE`) whose key already exists in `triple` (eid), `term` (id) or `tx` (t) SHALL be aborted by the database, regardless of which connection issues it. The check SHALL NOT depend on `PRAGMA recursive_triggers`.

#### Scenario: Replace of an existing statement
- **WHEN** a plain SQLite connection runs `INSERT OR REPLACE INTO triple(eid, s, p, o, t_add) VALUES (<e1>, …)` for an existing `e1`
- **THEN** the statement fails with `tiramemsu: eids are never reused`
- **AND** the row for `e1` is unchanged

#### Scenario: Replace of an existing term or transaction
- **WHEN** a plain SQLite connection runs `REPLACE INTO term …` with an existing id, or `REPLACE INTO tx …` with an existing `t`
- **THEN** the statement fails and the existing row is unchanged

#### Scenario: New rows are unaffected
- **WHEN** the engine inserts a statement with a freshly allocated eid
- **THEN** the insert succeeds

### Requirement: A statement is retracted at most once and its content never changes
An `UPDATE` on `triple` SHALL be aborted with the message `tiramemsu: only a single retraction is allowed` unless the old row is live (`t_ret` NULL), the new row sets `t_ret` to a non-NULL value, and `eid`, `s`, `p`, `o`, `t_add`, `v_from` and `v_to` are unchanged. Setting `t_ret` together with `ret_kind` on a live row SHALL be the only permitted update.

#### Scenario: Legal retraction
- **WHEN** a connection updates a live row, setting `t_ret = 9` and `ret_kind = 0`
- **THEN** the update succeeds

#### Scenario: Second retraction
- **WHEN** a connection updates a row whose `t_ret` is already 9, setting `t_ret = 12`
- **THEN** the update fails with `tiramemsu: only a single retraction is allowed` and `t_ret` stays 9

#### Scenario: Un-retracting
- **WHEN** a connection sets `t_ret = NULL` on a retracted row
- **THEN** the update fails

#### Scenario: Changing statement content
- **WHEN** a connection updates the `o`, `s`, `p`, `eid`, `t_add`, `v_from` or `v_to` column of any row, live or retracted, even while also setting `t_ret`
- **THEN** the update fails and the row is unchanged

#### Scenario: Setting only the retraction kind on a live row
- **WHEN** a connection sets `ret_kind = 1` on a live row without setting `t_ret`
- **THEN** the update fails

### Requirement: Terms are never deleted or changed
Any `DELETE` on `term` SHALL be aborted with `tiramemsu: terms are never deleted`, and any `UPDATE` on `term` SHALL be aborted with `tiramemsu: terms are immutable`.

#### Scenario: Raw term delete
- **WHEN** a plain SQLite connection deletes a row from `term`
- **THEN** the statement fails with `tiramemsu: terms are never deleted`

#### Scenario: Raw term rewrite
- **WHEN** a plain SQLite connection changes the `lex` of a term
- **THEN** the statement fails with `tiramemsu: terms are immutable`

### Requirement: Transactions are never deleted or changed
Any `DELETE` on `tx` SHALL be aborted with `tiramemsu: transactions are never deleted`, and any `UPDATE` on `tx` SHALL be aborted with `tiramemsu: transactions are immutable`.

#### Scenario: Raw transaction delete
- **WHEN** a plain SQLite connection deletes a row from `tx`
- **THEN** the statement fails with `tiramemsu: transactions are never deleted`

#### Scenario: Raw instant rewrite
- **WHEN** a plain SQLite connection changes the `instant` of a transaction
- **THEN** the statement fails with `tiramemsu: transactions are immutable`

### Requirement: The engine never issues deletions or rewrites
No engine code path SHALL issue `DELETE` against `triple`, `term` or `tx`, or an `UPDATE` of `triple` other than a single retraction. Forgetting a fact SHALL only be expressible as a retraction, so the as-of view at every transaction stays exact forever. Rolling back to a savepoint SHALL remain possible, because it is not a `DELETE`.

#### Scenario: Engine operations under the triggers
- **WHEN** a random sequence of every write operation (assert, create, retract, retract-matching, supersede, confirm, upsert, cardinality-one replacement, metadata, volatile upsert) runs against a database
- **THEN** no trigger ever fires an abort

#### Scenario: Savepoint rollback is not blocked
- **WHEN** a speculative transaction inserts statements and terms and then rolls back to its savepoint
- **THEN** the rollback succeeds and the inserted rows are gone

#### Scenario: Source audit
- **WHEN** the engine source is searched for SQL text containing `DELETE FROM triple`, `DELETE FROM term`, `DELETE FROM tx` or `UPDATE tx` / `UPDATE term`
- **THEN** no occurrence is found

### Requirement: Erasure never deletes rows
Format 1 SHALL offer no operation that erases a stored value; retraction is the only way to forget. Legal erasure is reserved for crypto-shredding (milestone M6), which destroys a key in the reserved `seal_key` table and SHALL NOT delete or rewrite any row of `triple`, `term` or `tx`. Format 1 SHALL NOT create a `seal_key` table.

#### Scenario: Retracted values stay in history
- **WHEN** a statement with a string object is retracted
- **THEN** its term remains in `term` and the statement remains visible in the history view and in the as-of view before the retraction

#### Scenario: Erasure name is reserved
- **WHEN** the schema of a format-1 database is inspected
- **THEN** it contains no object named `seal_key`

### Requirement: Volatile and metadata tables are outside the invariant
The `meta` and `volatile` tables SHALL NOT be protected by never-forget triggers, because counters must advance and volatile state is overwritten by design.

#### Scenario: Volatile row is overwritten
- **WHEN** a volatile value is set twice for the same subject and key
- **THEN** the second write replaces the first without error
