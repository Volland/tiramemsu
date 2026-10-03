## ADDED Requirements

### Requirement: Chunked import semantics
Each chunk SHALL execute atomically through the existing transaction engine and report committed counts and transaction ids. An invalid chunk SHALL roll back only that chunk.

#### Scenario: Later chunk fails
- **WHEN** two chunks commit and the third violates predicate schema
- **THEN** the first two remain visible and the third leaves no trace

#### Scenario: Retry an assertion chunk
- **WHEN** an idempotent assertion chunk is retried
- **THEN** existing facts are reused according to normal assertion semantics

### Requirement: Deferred maintenance
An import session SHALL suppress per-chunk analysis and run one full analysis with pooled-reader refresh on successful explicit finalization.

#### Scenario: Finalize many chunks
- **WHEN** twenty chunks complete and the import is finalized
- **THEN** no chunk triggers analysis and finalization performs one full analysis

#### Scenario: Analysis failure
- **WHEN** final analysis fails after chunks commit
- **THEN** the result identifies committed chunks and a maintenance error without claiming the data was rolled back

### Requirement: Progress and interruption
Progress SHALL distinguish committed rows, rejected chunks, and maintenance time. Cancellation or session drop SHALL release the import lease without discarding committed history.

#### Scenario: Cancel between chunks
- **WHEN** an import is cancelled after a committed chunk
- **THEN** the committed chunk is retained, statistics are marked due, and ordinary writes can resume

#### Scenario: Read while importing
- **WHEN** a reader queries while a chunk is uncommitted
- **THEN** it sees only the last committed snapshot

### Requirement: Exclusive write lease
An import session SHALL hold an exclusive write lease while it is open: any other write, or a second session, SHALL fail with a typed `ImportInProgress` error and commit nothing, while reads continue unaffected.

#### Scenario: Write during import
- **WHEN** an ordinary transaction, SPARQL update or Cypher write starts while a session is open
- **THEN** it fails with `ImportInProgress` and leaves no trace, and it succeeds once the session finishes or is cancelled
