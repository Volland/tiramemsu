## Status

Implemented. Rust: `Db::save_answer`/`save_answer_with`, `saved_answer`, `saved_answers`, `check_saved_answers`, `refresh_answer`/`refresh_answer_with`, `delete_saved_answer`, with `SavedQuery`, `SavedAnswer`, `AnswerStatus`, `CoverageReason`, `Invalidation`, `InvalidationCause` and `Error::SavedAnswerNotFound`. JSON bridge: `saveAnswer`, `savedAnswer`, `savedAnswers`, `checkSavedAnswers`, `refreshAnswer`, `deleteSavedAnswer`. Node: `Database.saveAnswer` and friends; Python: `Database.save_answer` and friends. MCP: `save_answer`, `saved_answers`, `check_answers`, `refresh_answer`. Storage format 3 adds the derived tables.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement saved answers and conservative invalidation behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec, and archive only after implementation is complete.
