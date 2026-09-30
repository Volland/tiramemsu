## MODIFIED Requirements

### Requirement: Dry-run preview of a cascade
A dry-run transaction SHALL report the complete set of statements every retraction would retract, with their kinds, while the `triple` and `tx` tables stay unchanged. Only the id counters in `meta` MAY advance. A read-only preview SHALL also be available on any view without the writer: `View::dependents(e)` on the now view SHALL list, as a set, exactly the statements and memberships that a dry-run retraction of the live statement `e` reports, as the `statement-dependents` capability specifies.

#### Scenario: Preview a large cascade
- **WHEN** a statement with 3 annotations and 2 references is retracted in a dry run
- **THEN** the report lists all 6 statements with their kinds
- **AND** all 6 statements are still live afterwards and no `tx` row was added

#### Scenario: Read-only preview
- **WHEN** the same statement's dependents are read on the now view
- **THEN** the result holds the same 6 statements, the root first
- **AND** no id counter in `meta` advances
