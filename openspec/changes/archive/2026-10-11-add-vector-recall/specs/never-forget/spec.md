## ADDED Requirements

### Requirement: Embeddings are never deleted or changed
From storage format 5, the `embedding` table SHALL be protected by the triggers `embedding_no_delete` and `embedding_no_update`, which abort any `DELETE` or `UPDATE` of its rows, because embeddings are caller-supplied source data that cannot be rebuilt from the graph. No engine code path SHALL issue `UPDATE` or `DELETE` against `embedding`, except the erase transaction of crypto-shredding, which removes the embedding rows of an erased subject's statements. Rolling back a transaction or savepoint that inserted embeddings SHALL remain possible.

#### Scenario: Raw delete refused
- **WHEN** a raw SQLite connection deletes a row of `embedding`
- **THEN** the trigger aborts the statement and the row remains

#### Scenario: Raw update refused
- **WHEN** a raw SQLite connection updates the `vec` of a row of `embedding`
- **THEN** the trigger aborts the statement and the row is unchanged

#### Scenario: Speculation rollback is not blocked
- **WHEN** a speculative transaction sets embeddings and then rolls back to its savepoint
- **THEN** the rollback succeeds and the inserted embedding rows are gone
