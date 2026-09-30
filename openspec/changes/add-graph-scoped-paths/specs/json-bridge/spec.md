## MODIFIED Requirements

### Requirement: Paths and graphs

`path` SHALL take `start`, `path` text, `mode` (`reach`, `trail`, `anyShortest`, `allShortest`), `maxHops` and an optional `graphs` list of terms, and return one row per endpoint with its hop count and, except in `reach` mode, its nodes and hops. With `graphs`, the path SHALL be evaluated with graph-scoped evaluation over those graphs; a listed term that is not stored names no graph. `addToGraph`, `graphs` and `graphMembers` SHALL expose named graphs as tags on statements.

#### Scenario: Reachability
- **WHEN** a knows b and b knows c, and `path` runs `knows+` from a
- **THEN** it returns two rows

#### Scenario: A graph holds a statement
- **WHEN** a statement is added to graph session12
- **THEN** `graphs` returns session12 and `graphMembers` returns the statement's eid

#### Scenario: Path inside a graph
- **WHEN** a knows b is in graph session12, b knows c is in no graph, and `path` runs `knows+` from a with `"graphs": [session12]`
- **THEN** it returns one row, for b
