## MODIFIED Requirements

### Requirement: tm_path arguments
`tm_path(start, path, mode, max_hops, view, graphs)` SHALL accept its arguments positionally:
- `start` (required): an ObjectId integer. The path is evaluated from this start.
- `path` (required): the path expression text. See "Path expression text".
- `mode` (optional): one of `REACH`, `TRAIL`, `ANY_SHORTEST`, `ALL_SHORTEST`, compared case-insensitively. Default `REACH`.
- `max_hops` (optional): a non-negative integer. When NULL or omitted, the default is no hop bound for `REACH`, `ANY_SHORTEST` and `ALL_SHORTEST`, and the database's path hop cap (default 15) for `TRAIL`.
- `view` (optional): the view text. See "View argument text". When NULL or omitted, the default is `now`.
- `graphs` (optional): the graph set of graph-scoped evaluation. NULL or omitted means no graph filter; an INTEGER is the ObjectId of one graph; a TEXT is a JSON array of integer ObjectIds (`'[]'` is the empty set). It MAY be a column of a preceding table, so a call can be correlated with a graph.

Trailing optional arguments MAY be omitted.

#### Scenario: Minimal call
- **WHEN** `SELECT "end" FROM tm_path(:alice, 'knows')` is run and `(alice knows bob)` is live
- **THEN** one row with `end = :bob` is returned

#### Scenario: Mode is case-insensitive
- **WHEN** `tm_path(:a, 'p+', 'any_shortest')` is run
- **THEN** it behaves exactly as `tm_path(:a, 'p+', 'ANY_SHORTEST')`

#### Scenario: Default trail cap
- **WHEN** `tm_path(:n0, 'next+', 'TRAIL')` is run over a 20-hop `next` chain with the default database cap
- **THEN** 15 rows are returned, with hops 1 through 15

#### Scenario: Explicit max_hops
- **WHEN** `tm_path(:n0, 'next+', 'TRAIL', 18)` is run over the same chain
- **THEN** 18 rows are returned

#### Scenario: One graph as an integer
- **WHEN** `(a knows b)` is a member of `g1`, `(b knows c)` is in no graph, and `tm_path(:a, 'knows+', 'REACH', NULL, NULL, :g1)` is run
- **THEN** the only end is `b`

#### Scenario: Several graphs as JSON text
- **WHEN** `(b knows c)` is a member of `g2` and `tm_path(:a, 'knows+', 'REACH', NULL, NULL, '[' || :g1 || ',' || :g2 || ']')` is run
- **THEN** the ends are `b` and `c`

#### Scenario: Graph correlated with a column
- **WHEN** a statement runs `SELECT m.o, p."end" FROM (SELECT DISTINCT o FROM triple WHERE p = :inGraph) m, tm_path(:a, 'knows+', 'REACH', NULL, NULL, m.o) p`
- **THEN** each row pairs a graph with an end reachable from `a` inside that graph

### Requirement: tm_path argument errors
`tm_path` SHALL fail the SQL statement with an error message that starts with `tm_path:` and names the offending argument when:
- `start` or `path` is missing;
- `start` is not an integer;
- `path` is not text, or does not parse;
- `mode` is not one of the four v1 modes;
- `max_hops` is negative or not an integer;
- `view` is malformed;
- `graphs` is neither NULL, an integer nor text, or is text that is not a JSON array of integers.

A NULL `start`, as produced by an outer join, SHALL produce zero rows and no error.

#### Scenario: Missing path argument
- **WHEN** `SELECT * FROM tm_path(:a)` is run
- **THEN** the statement fails with an error that starts with `tm_path:` and names `path`

#### Scenario: Unknown mode
- **WHEN** `SELECT * FROM tm_path(:a, 'p', 'WALK')` is run
- **THEN** the statement fails with an error that starts with `tm_path:` and names `mode`

#### Scenario: Negative max_hops
- **WHEN** `SELECT * FROM tm_path(:a, 'p', 'TRAIL', -1)` is run
- **THEN** the statement fails with an error that starts with `tm_path:` and names `max_hops`

#### Scenario: Malformed view
- **WHEN** `SELECT * FROM tm_path(:a, 'p', 'REACH', NULL, 'asOf/yesterday')` is run
- **THEN** the statement fails with an error that starts with `tm_path:` and names `view`

#### Scenario: Non-integer start
- **WHEN** `SELECT * FROM tm_path('alice', 'p')` is run
- **THEN** the statement fails with an error that starts with `tm_path:` and names `start`

#### Scenario: Malformed graphs
- **WHEN** `tm_path(:a, 'p', 'REACH', NULL, NULL, '[1, "g"]')`, `tm_path(:a, 'p', 'REACH', NULL, NULL, 'g1')` or `tm_path(:a, 'p', 'REACH', NULL, NULL, 1.5)` is run
- **THEN** the statement fails with an error that starts with `tm_path: graphs:`

#### Scenario: NULL start from an outer join
- **WHEN** `tm_path` is correlated to the nullable side of a `LEFT JOIN` and receives a NULL start
- **THEN** it contributes zero rows for that outer row, and the statement succeeds

### Requirement: View::path API
The Rust `View` handle SHALL offer `path_with(start, path, &PathArgs)`, where `PathArgs` holds the mode, the hop bound and an optional graph set and implements `Default` (`REACH`, no bound, no graph filter). It evaluates the path text from `start` under the view's own tx-time and valid-time selection, with graph-scoped evaluation when a graph set is given, and returns the rows as values with:
- `start`;
- `end`;
- `hops`;
- an optional path, with its node sequence and its hop sequence (eid, predicate, direction).

`path(start, path, mode, max_hops)` SHALL remain as a shorthand for `path_with` with no graph set. `max_hops` SHALL be a hard upper bound for every mode. A caller who wants no bound passes the largest representable value. The rows SHALL be in the deterministic order of the path evaluation. Errors SHALL be typed:
- a parse error in the `Path` dialect, for malformed path text;
- `Unsupported`, for an unsupported mode;
- `PathLimitExceeded { limit }`, when the search-state guard trips.

#### Scenario: API matches tm_path
- **WHEN** `db.as_of(Tx(15)).path(alice, "knows+", PathMode::Reach, u32::MAX)` is called, and `tm_path(:alice, 'knows+', 'REACH', NULL, 'asOf/15')` is run on the same database
- **THEN** both return the same `(start, end, hops)` rows in the same order

#### Scenario: API uses the view's valid time
- **WHEN** `db.now().valid_at(d).path(a, "worksAt/locatedIn", PathMode::Trail, 5)` is called
- **THEN** only hops through statements valid at `d` are taken

#### Scenario: API returns path values
- **WHEN** `db.now().path(a, "knows", PathMode::Trail, 1)` is called and `(a knows b)` has eid `e1`
- **THEN** the single row has path nodes `[a, b]` and one hop `(e1, knows, forward)`

#### Scenario: API parse error
- **WHEN** `db.now().path(a, "knows//", PathMode::Reach, 3)` is called
- **THEN** it returns a parse error in the `Path` dialect with a span, and no rows

#### Scenario: API inside speculation
- **WHEN** `db.with(ops, |view| view.path(...))` is called
- **THEN** the path sees the speculative statements, and nothing is persisted afterwards

#### Scenario: API with a graph set
- **WHEN** `db.now().path_with(a, "knows+", &PathArgs { graphs: Some(vec![g1]), ..PathArgs::default() })` is called, and `tm_path(:a, 'knows+', 'REACH', NULL, NULL, :g1)` is run on the same database
- **THEN** both return the same rows in the same order
