## MODIFIED Requirements

### Requirement: Native operator extension point
The planner SHALL split the IR into regions and route each PathPattern to a registered native path operator, which SQL reaches as the table-valued function `tm_path(start, path, mode, max_hops, view)` returning `(start, end, hops, path_json)`. The call SHALL receive the pattern's View in its `view` argument, and a start value that is a bound constant, a parameter or a column of an earlier FROM item. A database opened through `Db::open` registers the `tm_path` operator of the path engine on every connection. An engine constructed without a registered path operator SHALL fail a query containing a PathPattern with `Unsupported` before any SQL is executed. Cyclic basic graph patterns SHALL be detected and routed to SQL while the LFTJ extension point is disabled, which is the only setting in this change.

#### Scenario: No path operator registered
- **WHEN** an IR containing a PathPattern is prepared on a query engine with no path operator registered
- **THEN** execution fails with `Unsupported` naming path patterns, and no SQL is executed

#### Scenario: Path composes as a table-valued function
- **WHEN** a test path operator is registered and an IR joins `TriplePattern(?b, v:supportedBy, ?r)` with `PathPattern(?r, ?x, sys:subject*)`
- **THEN** the explained SQL contains a single statement with `tm_path(…)` as a FROM item whose start argument is the column bound to `?r`
- **AND** the view argument encodes the path pattern's View

#### Scenario: End-bound path
- **WHEN** a PathPattern has an unbound start and a constant end
- **THEN** the planner invokes the path operator from the bound end with the inverse path, and binds the start variable from its `end` column
- **AND** a path variable of the pattern is returned read from the start to the end of the pattern

#### Scenario: Triangle query without LFTJ
- **WHEN** a cyclic pattern `(?a knows ?b), (?b knows ?c), (?c knows ?a)` is executed
- **THEN** the explain output reports the region as cyclic and routed to SQL, and the result equals a brute-force enumeration
