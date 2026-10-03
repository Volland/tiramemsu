## ADDED Requirements

### Requirement: Saved answer identity
A saved answer SHALL retain query text or IR, parameters, view, result, dependency eids, coverage reasons, and event checkpoint sufficient for deterministic re-evaluation.

#### Scenario: Save a parameterized query
- **WHEN** an answer is saved
- **THEN** its original parameters and view can be recovered without relying on current defaults

### Requirement: Conservative freshness
An answer SHALL NOT be declared fresh solely because its positive dependency eids remain live. Inserts and incomplete coverage SHALL conservatively request re-evaluation when relevance cannot be excluded.

#### Scenario: Negative condition
- **WHEN** an answer used NOT EXISTS and a matching statement is inserted
- **THEN** the answer becomes stale or requires recheck

#### Scenario: Additional matching row
- **WHEN** a new statement adds a row to a saved current-view query
- **THEN** the saved answer is marked for re-evaluation even when all previous supporting eids remain live

#### Scenario: Retracted support
- **WHEN** a supporting statement is retracted or superseded
- **THEN** the answer is marked stale with the triggering event

### Requirement: Replayable checkpoints
Invalidation SHALL be idempotent and restartable from a persisted event checkpoint. Clearing stale status SHALL require successful re-evaluation, not merely acknowledging a notification.

#### Scenario: Restart event consumer
- **WHEN** processing resumes after a saved checkpoint
- **THEN** events are replayed without losing or duplicating logical invalidations

#### Scenario: Re-evaluation fails
- **WHEN** refreshing a saved answer fails
- **THEN** its prior result remains marked stale and its freshness checkpoint does not advance

### Requirement: Fixed historical views
An answer on a transaction-time view that can no longer change SHALL stay fresh across later events, unless its result depends on the clock.

#### Scenario: As-of answer after new events
- **WHEN** statements cited by an answer saved on a past as-of view are retracted and new statements are inserted
- **THEN** the answer stays fresh, while a clock-sensitive answer on the same view requires recheck once the clock has moved

### Requirement: Derived records
Saving, checking, refreshing and deleting saved answers SHALL NOT consume transaction numbers, add events, or change any statement, term or transaction row.

#### Scenario: History unchanged
- **WHEN** answers are saved, checked, refreshed and deleted
- **THEN** the event log, the counters and the graph rows are unchanged
