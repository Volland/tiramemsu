# path-table-function Specification

## Purpose
Defines the SQL table-valued function `tm_path` and the `View::path` Rust API entry. They expose native path evaluation to SQL, where paths compose with generated and hand-written SQL, and to API callers, with one shared contract for arguments, output and errors.

## Requirements

### Requirement: tm_path is available on every connection
The system SHALL make the table-valued function `tm_path` available, with no `CREATE VIRTUAL TABLE` statement, on every connection the database opens: every reader connection and the writer connection. It SHALL be read-only. Any attempt to insert, update or delete through it SHALL fail.

#### Scenario: Callable on a reader connection
- **WHEN** a query on a reader connection runs `SELECT * FROM tm_path(:alice, 'knows+', 'REACH')`
- **THEN** it returns the reachability rows from `alice`, with no prior setup

#### Scenario: Callable inside speculation
- **WHEN** a speculative `with` asserts `(bob knows carol)`, and within it a query runs `tm_path(:alice, 'knows+', 'REACH')` where `(alice knows bob)` is live
- **THEN** `carol` is among the ends, and after the speculation ends the same call no longer returns `carol`

#### Scenario: Writes are rejected
- **WHEN** a statement runs `INSERT INTO tm_path VALUES (1, 2, 3, NULL)`
- **THEN** the statement fails and the database is unchanged

### Requirement: tm_path arguments
`tm_path(start, path, mode, max_hops, view)` SHALL accept its arguments positionally:
- `start` (required): an ObjectId integer. The path is evaluated from this start.
- `path` (required): the path expression text. See "Path expression text".
- `mode` (optional): one of `REACH`, `TRAIL`, `ANY_SHORTEST`, `ALL_SHORTEST`, compared case-insensitively. Default `REACH`.
- `max_hops` (optional): a non-negative integer. When NULL or omitted, the default is no hop bound for `REACH`, `ANY_SHORTEST` and `ALL_SHORTEST`, and the database's path hop cap (default 15) for `TRAIL`.
- `view` (optional): the view text. See "View argument text". When NULL or omitted, the default is `now`.

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

### Requirement: Path expression text
The `path` argument, and the path text of `View::path`, SHALL use SPARQL 1.1 property path syntax: `/`, `|`, `*`, `+`, `?`, `^` and parentheses. It SHALL also accept a bounded repetition suffix `{m,n}`, `{m,}` or `{n}`, meaning Cypher `*m..n`, `*m..` or `*n`. A predicate atom SHALL be one of:
- a full IRI in angle brackets;
- a CURIE whose prefix is in the database prefix table or is a reserved prefix (`sys:`, `tm:`);
- a bare name, resolved through the database `@vocab` base exactly as Cypher names are resolved.

The reserved atom `sys:anyRelationship` SHALL denote the relationship-view wildcard step. Whitespace between tokens SHALL be ignored.

#### Scenario: Wildcard atom
- **WHEN** the store holds `(a knows b)`, `(a name "Ann")` and `(a rdf:type Person)`, and the path text `sys:anyRelationship` is evaluated from `a`
- **THEN** the only end is `b`

#### Scenario: Bare names resolve through vocab
- **WHEN** the default `@vocab` is in effect and the path text `SUPPORTED_BY/sys:subject` is used
- **THEN** it denotes the IRI `urn:tiramemsu:v:SUPPORTED_BY` followed by the virtual hop `sys:subject`

#### Scenario: Full IRI and CURIE atoms
- **WHEN** the prefix `schema` is mapped to `https://schema.org/` and the path text is `<https://schema.org/knows>|schema:follows`
- **THEN** it denotes the alternation of the two IRIs

#### Scenario: Bounded repetition suffix
- **WHEN** the path text `next{2,3}` is evaluated from `n0` over the `next` chain in `TRAIL` mode
- **THEN** exactly the ends `n2` and `n3` are returned

#### Scenario: Malformed path text
- **WHEN** the path text is `knows//likes` or `(knows`
- **THEN** the call fails with a parse error that names the path dialect and the character offset of the error

#### Scenario: Unknown prefix
- **WHEN** the path text is `nope:knows` and `nope` is not a known prefix
- **THEN** the call fails with a parse error naming the unknown prefix

### Requirement: View argument text
The `view` argument SHALL accept:
- `now`;
- `asOf/<t>`, where `<t>` is a transaction number;
- `asOf/<instant>`, where `<instant>` is an RFC 3339 date-time and is resolved as in `asOf(instant)`;
- `history`;
- any of the above followed by `;validAt/<d>`, where `<d>` is an RFC 3339 date or date-time;
- `validAt/<d>` alone, meaning `now;validAt/<d>`.

Each form SHALL also be accepted with the full `urn:tiramemsu:tm:` IRI prefix on each part.

#### Scenario: As of a transaction
- **WHEN** `(a knows b)` was asserted in tx 10, `(b knows c)` was asserted in tx 20, and `tm_path(:a, 'knows+', 'REACH', NULL, 'asOf/15')` is run
- **THEN** the only end is `b`

#### Scenario: History plus valid time
- **WHEN** `tm_path(:a, 'worksAt', 'REACH', NULL, 'history;validAt/2021-06-01')` is run
- **THEN** it returns the ends of every `worksAt` statement from `a` ever recorded whose valid interval contains 2021-06-01

#### Scenario: Full IRI form
- **WHEN** the view argument is `urn:tiramemsu:tm:asOf/15`
- **THEN** it behaves exactly as `asOf/15`

### Requirement: tm_path output columns
`tm_path` SHALL return exactly the visible columns `start`, `end`, `hops`, `path_json`, in this order, so `SELECT *` yields these four:
- `start`: the start ObjectId.
- `end`: the end ObjectId.
- `hops`: an integer hop count.
- `path_json`: NULL in `REACH` mode, and otherwise a JSON text of the form `{"nodes":[<ObjectId>,…],"edges":[{"eid":<ObjectId>,"p":<ObjectId>,"dir":"out"|"in"},…]}`.

In `path_json`, `nodes` lists hops + 1 node ids, from start to end. `edges` lists hops entries in traversal order. `dir` is `"out"` for a forward hop and `"in"` for an inverse hop. `p` is the virtual predicate's IRI id for virtual hops. The row order SHALL be the deterministic order of the path evaluation when the statement has no `ORDER BY`.

#### Scenario: Select star columns
- **WHEN** `SELECT * FROM tm_path(:a, 'p', 'TRAIL')` is run
- **THEN** the result has exactly the columns `start`, `end`, `hops`, `path_json`

#### Scenario: path_json for a trail
- **WHEN** `(a knows b)` has eid `e1` and `SELECT path_json FROM tm_path(:a, 'knows', 'TRAIL')` is run
- **THEN** the value parses as JSON, equal to `{"nodes":[:a,:b],"edges":[{"eid":e1,"p":<knows id>,"dir":"out"}]}` with the ObjectIds written as JSON integers

#### Scenario: path_json is NULL in REACH mode
- **WHEN** `SELECT path_json FROM tm_path(:a, 'knows+', 'REACH')` is run
- **THEN** every returned value is NULL

#### Scenario: Zero-length row
- **WHEN** `tm_path(:a, 'knows*', 'TRAIL')` is run
- **THEN** one row has `start = :a`, `end = :a`, `hops = 0` and `path_json = {"nodes":[:a],"edges":[]}`

### Requirement: tm_path composes with SQL
`tm_path` SHALL be usable as a `FROM`-clause item in any SQL statement. This includes a correlated call whose `start` comes from a column of a preceding table, and joins, filters, aggregates and `ORDER BY` over its columns. It SHALL support its SQL-level JSON functions over `path_json`. An equality constraint on `end` in the same statement SHALL restrict the rows to that end, and SHALL give the same rows as filtering afterwards.

#### Scenario: Correlated start from another table
- **WHEN** a statement runs `SELECT t.s, p."end" FROM triple t, tm_path(t.s, 'knows+', 'REACH') p WHERE t.p = :type AND t.o = :Person AND t.t_ret IS NULL`
- **THEN** it returns, for each live `Person`, every node reachable over `knows+`

#### Scenario: Aggregate over paths
- **WHEN** a statement runs `SELECT "end", count(*) FROM tm_path(:a, 'p+', 'ALL_SHORTEST') GROUP BY "end"`
- **THEN** each end is listed with its number of distinct shortest paths

#### Scenario: End constraint
- **WHEN** a statement runs `SELECT * FROM tm_path(:a, 'p+', 'ANY_SHORTEST') WHERE "end" = :d`
- **THEN** at most one row is returned, and it is the same row that the unconstrained call returns for `:d`

#### Scenario: JSON inspection of hops
- **WHEN** a statement runs `SELECT j.value ->> 'eid' FROM tm_path(:a, 'p{1,3}', 'TRAIL') p, json_each(p.path_json, '$.edges') j`
- **THEN** it returns the eid of every hop of every trail

### Requirement: tm_path argument errors
`tm_path` SHALL fail the SQL statement with an error message that starts with `tm_path:` and names the offending argument when:
- `start` or `path` is missing;
- `start` is not an integer;
- `path` is not text, or does not parse;
- `mode` is not one of the four v1 modes;
- `max_hops` is negative or not an integer;
- `view` is malformed.

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

#### Scenario: NULL start from an outer join
- **WHEN** `tm_path` is correlated to the nullable side of a `LEFT JOIN` and receives a NULL start
- **THEN** it contributes zero rows for that outer row, and the statement succeeds

### Requirement: tm_path reads a consistent snapshot
All neighbour reads of one `tm_path` evaluation SHALL come from the snapshot of the SQL statement that calls it. On a reader connection this is the read transaction's WAL snapshot. On the writer connection inside speculation it is the uncommitted speculative state.

#### Scenario: Concurrent commit is not seen mid-query
- **WHEN** a long `tm_path` evaluation runs on a reader, and a writer commits `(c knows d)` after the evaluation has started
- **THEN** that evaluation does not return `d` through the new statement

### Requirement: View::path API
The Rust `View` handle SHALL offer `path(start, path, mode, max_hops)`. It evaluates the path text from `start` under the view's own tx-time and valid-time selection, and returns the rows as values with:
- `start`;
- `end`;
- `hops`;
- an optional path, with its node sequence and its hop sequence (eid, predicate, direction).

`max_hops` SHALL be a hard upper bound for every mode. A caller who wants no bound passes the largest representable value. The rows SHALL be in the deterministic order of the path evaluation. Errors SHALL be typed:
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
