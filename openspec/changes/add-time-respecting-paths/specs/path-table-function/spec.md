## MODIFIED Requirements

### Requirement: View argument text
The `view` argument SHALL accept:
- `now`;
- `asOf/<t>`, where `<t>` is a transaction number;
- `asOf/<instant>`, where `<instant>` is an RFC 3339 date-time and is resolved as in `asOf(instant)`;
- `history`;
- any of the above followed by `;validAt/<d>`, where `<d>` is an RFC 3339 date or date-time;
- `validAt/<d>` alone, meaning `now;validAt/<d>`;
- any of the above with a further part `timeRespecting` or `timeRespecting/<t>`, where `<t>` is an RFC 3339 date or date-time or an integer of epoch milliseconds, which makes the evaluation time-respecting from `<t>` (from −∞ without it). The parts are separated by `;` in any order, each at most once; `timeRespecting` alone means `now;timeRespecting`.

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

#### Scenario: Time-respecting view parts
- **WHEN** the view argument is `timeRespecting`, `now;timeRespecting/2024-06-01`, `timeRespecting/1717200000000;asOf/15` or `urn:tiramemsu:tm:timeRespecting`
- **THEN** each is accepted, with the start instant −∞, 2024-06-01T00:00Z, 1717200000000 ms and −∞, and `timeRespecting;timeRespecting` or `timeRespecting/soon` fails with an error that starts with `tm_path: view:`

### Requirement: tm_path output columns
`tm_path` SHALL return exactly the visible columns `start`, `end`, `hops`, `path_json`, `arrival`, in this order, so `SELECT *` yields these five:
- `start`: the start ObjectId.
- `end`: the end ObjectId.
- `hops`: an integer hop count.
- `path_json`: NULL in `REACH` mode, and otherwise a JSON text of the form `{"nodes":[<ObjectId>,…],"edges":[{"eid":<ObjectId>,"p":<ObjectId>,"dir":"out"|"in"},…]}`.
- `arrival`: NULL unless the view text makes the evaluation time-respecting; then the row's arrival as an INTEGER of epoch milliseconds, or NULL when the arrival is −∞.

In `path_json`, `nodes` lists hops + 1 node ids, from start to end. `edges` lists hops entries in traversal order. `dir` is `"out"` for a forward hop and `"in"` for an inverse hop. `p` is the virtual predicate's IRI id for virtual hops. The row order SHALL be the deterministic order of the path evaluation when the statement has no `ORDER BY`.

#### Scenario: Select star columns
- **WHEN** `SELECT * FROM tm_path(:a, 'p', 'TRAIL')` is run
- **THEN** the result has exactly the columns `start`, `end`, `hops`, `path_json`, `arrival`

#### Scenario: path_json for a trail
- **WHEN** `(a knows b)` has eid `e1` and `SELECT path_json FROM tm_path(:a, 'knows', 'TRAIL')` is run
- **THEN** the value parses as JSON, equal to `{"nodes":[:a,:b],"edges":[{"eid":e1,"p":<knows id>,"dir":"out"}]}` with the ObjectIds written as JSON integers

#### Scenario: path_json is NULL in REACH mode
- **WHEN** `SELECT path_json FROM tm_path(:a, 'knows+', 'REACH')` is run
- **THEN** every returned value is NULL

#### Scenario: Zero-length row
- **WHEN** `tm_path(:a, 'knows*', 'TRAIL')` is run
- **THEN** one row has `start = :a`, `end = :a`, `hops = 0` and `path_json = {"nodes":[:a],"edges":[]}`

#### Scenario: Arrival of a time-respecting call
- **WHEN** `(a p b)` is valid `[1, 5)`, `(b p c)` is valid `[3, 9)`, and `SELECT "end", arrival FROM tm_path(:a, 'p+', 'REACH', NULL, 'timeRespecting')` is run
- **THEN** the rows are `(b, 1)` and `(c, 3)`, and without `timeRespecting` both arrivals are NULL

### Requirement: View::path API
The Rust `View` handle SHALL offer `path_with(start, path, &PathArgs)`, where `PathArgs` holds the mode, the hop bound, an optional graph set and an optional time-respecting option (`TimeRespecting { after }`) and implements `Default` (`REACH`, no bound, no graph filter, not time-respecting). It evaluates the path text from `start` under the view's own tx-time and valid-time selection, with graph-scoped evaluation when a graph set is given and time-respecting evaluation when the option is given, and returns the rows as values with:
- `start`;
- `end`;
- `hops`;
- `arrival`;
- an optional path, with its node sequence and its hop sequence (eid, predicate, direction).

`path(start, path, mode, max_hops)` SHALL remain as a shorthand for `path_with` with no graph set and no time-respecting option. `max_hops` SHALL be a hard upper bound for every mode. A caller who wants no bound passes the largest representable value. The rows SHALL be in the deterministic order of the path evaluation. Errors SHALL be typed:
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

#### Scenario: API time-respecting
- **WHEN** `db.now().path_with(a, "p+", &PathArgs { time_respecting: Some(TimeRespecting { after: Some(2) }), ..PathArgs::default() })` is called, and `tm_path(:a, 'p+', 'REACH', NULL, 'timeRespecting/2')` is run on the same database
- **THEN** both return the same ends, hops and arrivals in the same order
