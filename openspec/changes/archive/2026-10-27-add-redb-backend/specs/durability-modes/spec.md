## ADDED Requirements

### Requirement: Durability option
`OpenOptions::durability` SHALL accept `Normal` (default), `Immediate` and `None` on both backends. `Immediate` SHALL make every committed transaction survive a process crash. `Normal` and `None` SHALL leave a consistent database after a crash, possibly without the newest transactions; `Normal` SHALL bound that loss by the flush interval and commit count. `Db::sync()` SHALL make every committed transaction durable.

#### Scenario: Immediate survives a crash
- **WHEN** a child process opens a redb database with `Immediate`, commits a transaction and aborts
- **THEN** reopening the file shows the transaction

#### Scenario: Normal is consistent after a crash
- **WHEN** a child process opens a redb database with `Normal`, commits transactions and aborts without closing
- **THEN** reopening succeeds and the file holds a prefix of the committed transactions with gap-free numbering

#### Scenario: Sync makes commits durable
- **WHEN** a child process with `None` commits, calls `Db::sync()` and aborts
- **THEN** reopening shows every transaction committed before the sync
