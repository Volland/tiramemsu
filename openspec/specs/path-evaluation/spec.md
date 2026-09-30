# path-evaluation Specification

## Purpose
Defines how the database evaluates path queries natively and with awareness of time. It covers path expressions, the four v1 path modes, endpoint and hop limits, termination and memory guards, evaluation under every temporal view, and the shape and order of path results. SPARQL, Cypher, the API and SQL all share this behaviour.

## Requirements

### Requirement: Path expression operators
The system SHALL evaluate path expressions built from these parts:
- predicate atoms;
- sequence (`/`);
- alternation (`|`);
- zero-or-more (`*`);
- one-or-more (`+`);
- zero-or-one (`?`);
- inverse (`^`);
- bounded repetition with a minimum and an optional maximum number of repetitions;
- parentheses for grouping.

A forward atom `p` SHALL step from a node `x` to `y` for each statement `(x p y)` that is visible in the view. An inverse atom `^p` SHALL step from `y` to `x` over the same statements. Operator precedence SHALL be the SPARQL 1.1 property path precedence: `^` and the postfix operators bind tighter than `/`, and `/` binds tighter than `|`.

#### Scenario: Single forward predicate
- **WHEN** the store holds `(a knows b)` and `(b knows c)` and the path `knows` is evaluated from `a`
- **THEN** the only end is `b` with 1 hop

#### Scenario: Sequence
- **WHEN** the store holds `(a knows b)`, `(b worksAt acme)` and `(a worksAt globex)`, and the path `knows/worksAt` is evaluated from `a`
- **THEN** the only end is `acme` with 2 hops

#### Scenario: Alternation of predicates
- **WHEN** the store holds `(a knows b)`, `(a likes c)` and `(a hates d)`, and the path `knows|likes` is evaluated from `a`
- **THEN** the ends are exactly `b` and `c`

#### Scenario: Inverse step
- **WHEN** the store holds `(b knows a)` and the path `^knows` is evaluated from `a`
- **THEN** the only end is `b` with 1 hop

#### Scenario: One or more
- **WHEN** the store holds the chain `(a knows b)`, `(b knows c)`, `(c knows d)` and the path `knows+` is evaluated from `a` in reachability mode
- **THEN** the ends are exactly `b`, `c` and `d`, and `a` is not an end

#### Scenario: Zero or more includes the start
- **WHEN** the same chain is evaluated with the path `knows*` from `a` in reachability mode
- **THEN** the ends are exactly `a` (0 hops), `b`, `c` and `d`

#### Scenario: Zero or one
- **WHEN** the same chain is evaluated with the path `knows?` from `a` in reachability mode
- **THEN** the ends are exactly `a` (0 hops) and `b` (1 hop)

#### Scenario: Bounded repetition
- **WHEN** the same chain is evaluated with `knows` repeated between 2 and 3 times from `a`
- **THEN** the ends are exactly `c` (2 hops) and `d` (3 hops)

#### Scenario: Bounded repetition with open maximum
- **WHEN** the same chain is evaluated with `knows` repeated at least 2 times and no maximum, from `a`, in trail mode with `max_hops` 15
- **THEN** the ends are exactly `c` and `d`

#### Scenario: Precedence of inverse and sequence
- **WHEN** the store holds `(b knows a)` and `(b worksAt acme)`, and the path `^knows/worksAt` is evaluated from `a`
- **THEN** the only end is `acme`, because the expression parses as `(^knows)/worksAt`

#### Scenario: Grouped repetition of a sequence
- **WHEN** the store holds `(a p b)`, `(b q c)`, `(c p d)` and `(d q e)`, and the path `(p/q)+` is evaluated from `a` in reachability mode
- **THEN** the ends are exactly `c` and `e`

#### Scenario: Unknown predicate matches nothing
- **WHEN** a path mentions a predicate IRI that is not in the term dictionary, e.g. `neverUsed*` evaluated from `a`
- **THEN** evaluation succeeds without error, and only the zero-length match `a` is returned

### Requirement: Relationship-view wildcard step
The system SHALL support a wildcard step that matches any statement the Cypher relationship view exposes as a relationship. This means statements whose object is a node or a statement, or whose predicate has `sys:isEdge true`. It SHALL NOT match:
- literal-valued properties (unless `sys:isEdge true`);
- `rdf:type` label statements;
- statements whose predicate is in the reserved `sys:` namespace.

A wildcard step SHALL NOT match virtual layer hops. It SHALL also be usable inverted.

#### Scenario: Wildcard skips properties and labels
- **WHEN** the store holds `(a knows b)`, `(a name "Ann")`, `(a rdf:type Person)` and `(a sys:confirmedBy tx5)`, and a single wildcard step is evaluated from `a`
- **THEN** the only end is `b`

#### Scenario: Wildcard honours isEdge
- **WHEN** predicate `homepage` carries `sys:isEdge true`, the store holds `(a homepage "http://x")`, and a single wildcard step is evaluated from `a`
- **THEN** the literal `"http://x"` is an end

### Requirement: Path modes
The system SHALL support exactly four path modes in v1:
- `REACH`: reachability, endpoints only, set semantics.
- `TRAIL`: every path in which no relationship identity repeats.
- `ANY_SHORTEST`: one shortest matching path per distinct end.
- `ALL_SHORTEST`: every distinct shortest matching path per distinct end.

Any other mode, including `WALK`, `SIMPLE`, `ACYCLIC` and `SHORTEST k`, SHALL be rejected with an `Unsupported` error that names the mode.

#### Scenario: Unsupported mode is rejected
- **WHEN** a path is requested with mode `SIMPLE`
- **THEN** evaluation fails with an `Unsupported` error naming `SIMPLE`, and no rows are produced

### Requirement: Reachability mode semantics
In `REACH` mode the system SHALL return each distinct end reachable from a start through a word of the path language exactly once, whatever the number of distinct witnessing paths or statement eids. The reported hop count SHALL be the length of a shortest witnessing path. No path value SHALL be produced. For `*`, `?` and repetitions with minimum 0, the start SHALL match itself with 0 hops, even when the start appears in no statement.

#### Scenario: Set semantics over parallel edges
- **WHEN** the store holds two live statements `(a knows b)` with different eids and the path `knows+` is evaluated from `a` in `REACH` mode
- **THEN** `b` is returned exactly once

#### Scenario: Set semantics over diamond
- **WHEN** the store holds `(a p b)`, `(a p c)`, `(b p d)` and `(c p d)`, and the path `p+` is evaluated from `a` in `REACH` mode
- **THEN** `d` is returned exactly once with 2 hops

#### Scenario: Hops is shortest witness length
- **WHEN** the store holds `(a p b)`, `(b p c)` and `(a p c)`, and the path `p+` is evaluated from `a` in `REACH` mode
- **THEN** `c` is returned once with 1 hop

#### Scenario: Zero-length match of an isolated start
- **WHEN** the path `knows*` is evaluated from the IRI `nobody`, which appears in no statement, in `REACH` mode
- **THEN** exactly one row is returned, with end `nobody` and 0 hops

#### Scenario: Zero-length match of a literal start
- **WHEN** the path `knows*` is evaluated from the literal `"x"` in `REACH` mode
- **THEN** exactly one row is returned, with end `"x"` and 0 hops

### Requirement: Trail mode semantics
In `TRAIL` mode the system SHALL return one row per distinct path whose hop sequence matches the path expression and in which no relationship identity occurs twice. The relationship identity of a stored-statement hop SHALL be its eid, whatever the traversal direction. Nodes MAY repeat. Distinct statements with the same `(s, p, o)` SHALL be distinct relationships, so they give distinct paths.

#### Scenario: Parallel edges give distinct trails
- **WHEN** the store holds two live statements `(a knows b)` with eids `e1` and `e2`, and the path `knows` repeated 1 to 1 times is evaluated from `a` in `TRAIL` mode
- **THEN** two rows are returned, one through `e1` and one through `e2`

#### Scenario: A relationship is not reused
- **WHEN** the store holds the single statement `(a knows b)` (eid `e1`) and the path `(knows|^knows)` repeated 1 to 3 times is evaluated from `a` in `TRAIL` mode
- **THEN** exactly one row is returned, the 1-hop path `a -e1-> b`, because returning to `a` would reuse `e1`

#### Scenario: Nodes may repeat on a trail
- **WHEN** the store holds `(a p b)` (e1), `(b p a)` (e2) and `(a p c)` (e3), and the path `p+` is evaluated from `a` in `TRAIL` mode with `max_hops` 15
- **THEN** the returned paths include `a -e1-> b -e2-> a -e3-> c`, and no returned path contains the same eid twice

#### Scenario: Zero-length trail
- **WHEN** the path `knows*` is evaluated from `a` in `TRAIL` mode
- **THEN** one returned row has 0 hops, a path containing only the node `a`, and no relationships

### Requirement: Shortest path modes
In `ANY_SHORTEST` mode the system SHALL return, for each distinct end, exactly one path of minimal hop count among the paths that match the expression. It SHALL be the path whose hop-key sequence is smallest under the ordering defined in "Deterministic result ordering". In `ALL_SHORTEST` mode the system SHALL return, for each distinct end, every distinct path of minimal hop count, each exactly once. Two paths are distinct when their node or hop sequences differ.

#### Scenario: Any shortest picks the minimal length
- **WHEN** the store holds `(a p b)`, `(b p c)` and `(a p c)`, and the path `p+` is evaluated from `a` in `ANY_SHORTEST` mode
- **THEN** the row for end `c` has 1 hop and the path `a -> c`

#### Scenario: Any shortest is deterministic among ties
- **WHEN** the store holds `(a p b)` with eid `e5` and `(a p b)` with eid `e3`, and the path `p` is evaluated from `a` in `ANY_SHORTEST` mode
- **THEN** exactly one row is returned, its path goes through `e3`, and repeated evaluation returns the same row

#### Scenario: All shortest returns every minimal path
- **WHEN** the store holds `(a p b)`, `(a p c)`, `(b p d)`, `(c p d)` and `(a q x)`, `(x q y)`, `(y q d)`, and the path `(p|q)+` is evaluated from `a` in `ALL_SHORTEST` mode
- **THEN** for end `d` exactly two rows are returned, `a -> b -> d` and `a -> c -> d`, both with 2 hops, and the 3-hop path through `x` and `y` is not returned

#### Scenario: All shortest does not duplicate ambiguous matches
- **WHEN** the store holds `(a p b)` and the path `p|p` is evaluated from `a` in `ALL_SHORTEST` mode
- **THEN** exactly one row is returned for end `b`

#### Scenario: Shortest with both endpoints fixed
- **WHEN** the path `p+` is evaluated from `a` in `ANY_SHORTEST` mode with the end restricted to `d`
- **THEN** at most one row is returned, and its end is `d`

### Requirement: At least one bound endpoint
The system SHALL evaluate a path only when its start or its end is bound to a value at evaluation time. The value can come from a constant, a parameter, or a variable bound by another part of the query. When neither endpoint can be bound, the request SHALL fail with an `Unsupported` error saying that a path needs a bound endpoint. When only the end is bound, the system SHALL evaluate the path from that end and return rows whose start and end are the pattern's start and end, with paths in start-to-end order.

#### Scenario: Both endpoints unbound
- **WHEN** a query asks for all pairs `(x, y)` connected by `knows+`, and neither `x` nor `y` is bound by anything else in the query
- **THEN** the query fails with an `Unsupported` error that names the bound-endpoint requirement

#### Scenario: Only the end is bound
- **WHEN** the store holds `(a knows b)` and `(b knows c)`, and a path `knows+` is requested with the end bound to `c` and the start unbound, in `TRAIL` mode
- **THEN** the rows have start `b` and start `a`, each with end `c`, and the path for start `a` lists the nodes `a, b, c` in that order

### Requirement: Hop limits
The system SHALL honour a `max_hops` bound: no returned path, and no hop count reported in `REACH` mode, SHALL exceed it. When a pattern has no explicit upper bound and comes from Cypher, the system SHALL apply a cap that defaults to 15 hops and is configurable per database. Reaching the cap SHALL stop the extension of paths without an error: the paths within the cap are returned. An explicit finite upper bound in a Cypher pattern SHALL be honoured as written, even when it exceeds the default cap. SPARQL property paths SHALL be evaluated without a hop cap.

#### Scenario: Unbounded Cypher pattern is capped
- **WHEN** the store holds a `next` chain of 20 hops from `n0` to `n20`, and the Cypher pattern `(n0)-[:next*]->(x)` is evaluated with the default cap
- **THEN** exactly 15 rows are returned, for `n1` through `n15`, and no error is raised

#### Scenario: Configured cap
- **WHEN** the database is opened with a path hop cap of 5 and the same pattern is evaluated
- **THEN** exactly 5 rows are returned, for `n1` through `n5`

#### Scenario: Explicit bound above the cap
- **WHEN** the Cypher pattern `(n0)-[:next*1..18]->(x)` is evaluated with the default cap
- **THEN** 18 rows are returned, for `n1` through `n18`

#### Scenario: max_hops bounds reachability
- **WHEN** the path `next+` is evaluated from `n0` in `REACH` mode with `max_hops` 3
- **THEN** the ends are exactly `n1`, `n2` and `n3`

#### Scenario: SPARQL has no cap
- **WHEN** the SPARQL query `SELECT ?x WHERE { :n0 :next+ ?x }` is run over the 20-hop chain
- **THEN** 20 solutions are returned, `n1` through `n20`

### Requirement: Termination on cycles
Evaluation SHALL terminate in every mode on graphs with cycles, self-loops and statement-layer cycles, without an explicit hop bound in `REACH`, `ANY_SHORTEST` and `ALL_SHORTEST` modes.

#### Scenario: Reachability on a cycle
- **WHEN** the store holds `(a p b)`, `(b p c)` and `(c p a)`, and the path `p*` is evaluated from `a` in `REACH` mode with no hop bound
- **THEN** evaluation terminates and returns exactly `a`, `b` and `c`

#### Scenario: Self-loop
- **WHEN** the store holds `(a p a)` and the path `p+` is evaluated from `a` in `REACH` mode
- **THEN** evaluation terminates and returns `a` once, with 1 hop

#### Scenario: Trail on a cycle
- **WHEN** the same 3-cycle is evaluated with `p+` from `a` in `TRAIL` mode with `max_hops` 15
- **THEN** evaluation terminates and returns exactly three paths, of 1, 2 and 3 hops, the last ending at `a`

#### Scenario: Shortest on a cycle
- **WHEN** the same 3-cycle is evaluated with `p+` from `a` in `ALL_SHORTEST` mode with no hop bound
- **THEN** evaluation terminates, and returns one path each to `b` (1 hop), `c` (2 hops) and `a` (3 hops)

### Requirement: Search memory guard
The system SHALL bound the in-memory search state of a single path evaluation by a configurable limit (default 1 000 000 search states). When the limit would be exceeded, the evaluation SHALL fail with a `PathLimitExceeded` error that carries the limit. It SHALL NOT return a silently truncated result, and it SHALL leave the database unchanged.

#### Scenario: Guard trips on explosive trails
- **WHEN** the search-state limit is set to 1 000, and a `TRAIL` evaluation over a complete graph of 20 nodes with `max_hops` 10 needs more search states than that
- **THEN** evaluation fails with `PathLimitExceeded { limit: 1000 }`, and no partial rows are delivered to the caller of the query API

### Requirement: Time-aware evaluation
Every hop of a path SHALL read statements through the same view as the path pattern, with exactly the view semantics of triple-pattern scans:
- `Now`: live statements.
- `AsOf(t)`: statements with `t_add ≤ t` and not retracted at or before `t`.
- `History`: every statement ever recorded.
- `ValidAt(d)`, combined with any of the above: statements whose valid interval contains `d`.

One path evaluation SHALL use one view for all of its hops.

#### Scenario: Path valid now but not as of an earlier transaction
- **WHEN** `(a knows b)` is asserted in tx 10, `(b knows c)` is asserted in tx 20, and `knows+` is evaluated from `a` in `REACH` mode
- **THEN** under `Now` the ends are `b` and `c`, and under `AsOf(15)` the only end is `b`

#### Scenario: Path valid as of an earlier transaction but not now
- **WHEN** `(a knows b)` and `(b knows c)` are asserted in tx 10, `(b knows c)` is retracted in tx 30, and `knows+` is evaluated from `a`
- **THEN** under `AsOf(20)` the ends are `b` and `c`, and under `Now` the only end is `b`

#### Scenario: History traverses retracted statements
- **WHEN** the store is as in the previous scenario and `knows+` is evaluated from `a` under `History`
- **THEN** the ends are `b` and `c`

#### Scenario: Valid time filters hops
- **WHEN** `(a worksAt acme)` is valid `[2020-01-01, 2022-01-01)`, `(acme locatedIn berlin)` has no valid time, and `worksAt/locatedIn` is evaluated from `a`
- **THEN** under `ValidAt(2021-06-01)` the end is `berlin`, and under `ValidAt(2023-06-01)` no row is returned

#### Scenario: Superseded edge under asOf
- **WHEN** `(a worksAt acme)` (e1) is superseded in tx 40 by `(a worksAt globex)` (e10), and the path `worksAt` is evaluated from `a` in `TRAIL` mode
- **THEN** under `AsOf(39)` the only path goes through `e1` to `acme`, and under `Now` the only path goes through `e10` to `globex`

#### Scenario: Historical path results are stable
- **WHEN** a path result under `AsOf(t)` is computed, later transactions retract and supersede statements on that path, and the same path is evaluated again under `AsOf(t)`
- **THEN** both results are identical

### Requirement: Path result values
Each result row SHALL carry the start, the end and the hop count. In `TRAIL`, `ANY_SHORTEST` and `ALL_SHORTEST` modes, each row SHALL also carry a path value. The path value is the ordered node sequence, from the start to the end (hops + 1 nodes), and the ordered hop sequence (hops entries). Each hop entry records:
- the eid of the statement traversed;
- its predicate;
- whether it was traversed forward (subject to object) or inverse.

Nodes SHALL be reported as the same values that triple-pattern queries return for them. This includes statement eids that appear as nodes.

#### Scenario: Path value contents
- **WHEN** the store holds `(a knows b)` with eid `e1` and `(c knows b)` with eid `e2`, and the path `knows/^knows` is evaluated from `a` in `TRAIL` mode
- **THEN** the row has start `a`, end `c` and hops 2, the nodes `[a, b, c]`, and the hops `[(e1, knows, forward), (e2, knows, inverse)]`

#### Scenario: Reachability rows carry no path
- **WHEN** a path is evaluated in `REACH` mode
- **THEN** each row carries start, end and hops, and no path value

### Requirement: Deterministic result ordering
For a given database state, view, start, expression, mode and bound, the rows SHALL be produced in the same order every time. Rows SHALL be produced in non-decreasing hop count. Within one hop count:
- `REACH` rows SHALL be ordered by the end's raw ObjectId. This order exists only for determinism and is not a value order (for example, it is not dictionary lexical order for IRIs).
- Rows of the path-returning modes SHALL be ordered by their hop-key sequence, compared lexicographically. A hop key is ordered by eid, then by hop kind (stored statement before virtual hop), then by direction (forward before inverse).

#### Scenario: Reachability order
- **WHEN** the store holds `(a p c)`, `(a p b)` and `(b p d)`, and `p+` is evaluated from `a` in `REACH` mode
- **THEN** the rows for `b` and `c` (both 1 hop) come first, ordered by their ObjectIds, then `d` (2 hops)

#### Scenario: Trail order
- **WHEN** the store holds `(a p b)` with eid `e7` and `(a p c)` with eid `e4`, and `p` is evaluated from `a` in `TRAIL` mode
- **THEN** the path through `e4` comes before the path through `e7`

#### Scenario: Repeatable order
- **WHEN** the same path request is evaluated twice against the same view
- **THEN** both evaluations return identical row sequences
