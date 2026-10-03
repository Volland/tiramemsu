# memory-conflict-review Specification

## Purpose
TBD - created by archiving change add-memory-conflict-review. Update Purpose after archive.

## Requirements

### Requirement: View-scoped disagreement
Conflict inspection SHALL report subject/predicate pairs with distinct objects and overlapping valid intervals, along with source evidence and the selected transaction view.

#### Scenario: Disjoint valid times
- **WHEN** different objects apply in nonoverlapping valid intervals
- **THEN** they are not reported as simultaneous disagreement

#### Scenario: Parallel equal objects
- **WHEN** different statement ids have the same object
- **THEN** they are treated as supporting evidence rather than conflicting values

#### Scenario: Attributed evidence
- **WHEN** a conflict is reported
- **THEN** each value lists its statements with their ids, valid time, asserting transaction, confirmations, authors, sources and stated confidence, absent layers reported as absent and no combined score

#### Scenario: Multi-valued predicate
- **WHEN** the disagreeing predicate is declared multi-valued
- **THEN** the conflict is reported as potential disagreement and flagged as declared multi-valued, never as a schema violation

#### Scenario: History view
- **WHEN** conflict inspection runs on the history view
- **THEN** it fails with an unsupported error, because that view mixes statements never believed together

### Requirement: Noncommitting import preview
A bundle preview SHALL execute existing validation and write semantics without committing graph data. Its result SHALL disclose burned ids and preview scope.

#### Scenario: Invalid schema
- **WHEN** a previewed bundle violates schema
- **THEN** the preview reports the violation and writes no graph rows

#### Scenario: Successful preview
- **WHEN** a bundle preview succeeds
- **THEN** the caller sees proposed and reused facts while history and the event log remain unchanged

### Requirement: Explicit resolution and revalidation
Resolution or import application SHALL require an explicit write request and SHALL revalidate against current state. The system SHALL NOT claim synchronization or silently choose a source.

#### Scenario: State changed after preview
- **WHEN** another write changes a uniqueness constraint before application
- **THEN** application revalidates and either commits atomically or returns a schema error

#### Scenario: Inspect only
- **WHEN** conflict inspection runs
- **THEN** it does not retract, supersede, or confirm any fact
