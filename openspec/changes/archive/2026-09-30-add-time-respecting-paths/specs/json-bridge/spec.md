## MODIFIED Requirements

### Requirement: Paths and graphs

`path` SHALL take `start`, `path` text, `mode` (`reach`, `trail`, `anyShortest`, `allShortest`), `maxHops`, an optional `graphs` list of terms and an optional `timeRespecting` (`true`, or `{"after": <time>}` with a time as the bridge encodes times), and return one row per endpoint with its hop count, its `arrival` (epoch ms, or `null` when the search is not time-respecting or the arrival is unbounded) and, except in `reach` mode, its nodes and hops. With `graphs`, the path SHALL be evaluated with graph-scoped evaluation over those graphs; a listed term that is not stored names no graph. With `timeRespecting`, the path SHALL be evaluated time-respecting. `addToGraph`, `graphs` and `graphMembers` SHALL expose named graphs as tags on statements.

#### Scenario: Reachability
- **WHEN** a knows b and b knows c, and `path` runs `knows+` from a
- **THEN** it returns two rows

#### Scenario: A graph holds a statement
- **WHEN** a statement is added to graph session12
- **THEN** `graphs` returns session12 and `graphMembers` returns the statement's eid

#### Scenario: Path inside a graph
- **WHEN** a knows b is in graph session12, b knows c is in no graph, and `path` runs `knows+` from a with `"graphs": [session12]`
- **THEN** it returns one row, for b

#### Scenario: Time-respecting path
- **WHEN** a met b valid `[1, 5)`, b met c valid `[3, 9)`, and `path` runs `met+` from a with `"timeRespecting": true`
- **THEN** it returns b with `"arrival": 1` and c with `"arrival": 3`, and with `"timeRespecting": {"after": 6}` it returns no row
