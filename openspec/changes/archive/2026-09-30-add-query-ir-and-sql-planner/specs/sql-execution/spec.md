## Purpose

Defines how an IR query is planned into regions, compiled into a single parameterised SQL statement, executed on the right SQLite connection, and decoded into typed values. SPARQL and Cypher semantics are both honoured through the query's semantic flags, and native operators such as paths plug in as table-valued functions.

## ADDED Requirements

### Requirement: One parameterised SQL statement per query
The executor SHALL compile each non-empty IR query into exactly one SQL statement. Native-operator regions SHALL appear in it as table-valued-function FROM items. All data SHALL be bound as SQL parameters: IRIs, literals, ObjectIds, user parameters, AsOf transaction numbers, valid-time instants, path expressions, and skip and limit. The generated SQL text SHALL contain no data values. For the same IR shape and semantic flags, the SQL text SHALL be byte-identical, and alias names and parameter numbering SHALL be deterministic.

#### Scenario: Data never appears in SQL text
- **WHEN** an IR with the string constant `"x' OR 1=1 --"`, the IRI `v:alice` and `AsOf(Tx(42))` is explained
- **THEN** the SQL text contains none of `x' OR 1=1`, `v:alice`, the IRI's ObjectId, or `42`
- **AND** executing the query returns only rows whose object equals that string

#### Scenario: Stable text across constant values
- **WHEN** two IRs that differ only in constant values (for example `v:alice` versus `v:bob`) are explained
- **THEN** their SQL texts are byte-identical

#### Scenario: Golden SQL
- **WHEN** the golden corpus of IRs (single pattern per view, star join, chain join, layer join, optional, union, filter, aggregate, order/limit, values, virtual predicate, isomorphism, path TVF) is explained
- **THEN** each SQL text equals its checked-in snapshot

### Requirement: Triple pattern compilation
Each TriplePattern SHALL compile to its own alias of the triple table. A constant in the subject, predicate or object position SHALL become an equality with a bound parameter. A variable that occurs in several positions or patterns SHALL become equality between the corresponding columns. The pattern's view predicates SHALL be attached to its alias (see view-scoped-scans).

#### Scenario: Shared variable across patterns
- **WHEN** an IR `Join[TriplePattern(?a, v:worksAt, ?c), TriplePattern(?c, v:locatedIn, ?city)]` is executed
- **THEN** each row pairs a statement and a second statement whose subject equals the first one's object

#### Scenario: Repeated variable in one pattern
- **WHEN** an IR `TriplePattern(?x, v:knows, ?x)` is executed
- **THEN** only self-loops are returned

#### Scenario: Variable predicate
- **WHEN** an IR `TriplePattern(v:alice, ?p, ?o)` is executed under Now
- **THEN** every live stored triple with subject alice is returned, one row per statement under BagOfEids

### Requirement: Operator compilation
The executor SHALL give each operator these semantics. Join is the natural join on shared variables. LeftJoin keeps every left row: it extends the row with each compatible right row that satisfies the optional condition, and leaves the right-only variables missing when there is none. Filter keeps the rows whose condition is true. Union is the bag union, with variables absent from a branch left missing. Extend adds a computed variable. Aggregate groups by its keys and computes its aggregates. Project restricts and orders the columns, and removes duplicate rows when `distinct` is set. OrderLimit sorts by its keys, then skips and limits. Values produces its rows. The LeftJoin condition SHALL be evaluated as part of the join, not as a filter after it.

#### Scenario: Optional condition inside the join
- **WHEN** an IR `LeftJoin(TriplePattern(?p, v:name, ?n), TriplePattern(?p, v:age, ?a), cond = ?a > 30)` is executed and bob is 25
- **THEN** bob is returned with `?a` missing, and is not removed

#### Scenario: Union keeps duplicates
- **WHEN** both branches of a Union produce the row `?x = alice`
- **THEN** the result contains that row twice

#### Scenario: Distinct projection
- **WHEN** an IR `Project{distinct}[?c](TriplePattern(?a, v:worksAt, ?c))` is executed and three people work at acme
- **THEN** acme appears once

#### Scenario: Skip without limit
- **WHEN** an IR `OrderLimit(keys = [?n], skip = 2, limit = none)` is executed over 5 rows
- **THEN** the last 3 rows in order are returned

#### Scenario: Limit zero
- **WHEN** an IR `OrderLimit(limit = 0)` is executed
- **THEN** zero rows are returned with the correct columns

### Requirement: Relationship isomorphism
Under `match_mode = RelIsomorphism`, two TriplePatterns that are marked as relationship patterns of the same match group SHALL never bind the same statement in one result row. Under `match_mode = Homomorphism`, no such constraint SHALL apply. Patterns without a match group, or in different match groups, SHALL never be constrained against each other.

#### Scenario: A self-loop cannot be traversed twice
- **WHEN** the only data is e1 = `(a knows a)`, and an IR matches `(?x knows ?y, eid ?r1), (?y knows ?z, eid ?r2)` as relationship patterns of one group under RelIsomorphism with BagOfEids
- **THEN** the result is empty

#### Scenario: Same pattern under homomorphism
- **WHEN** the same IR runs under Homomorphism
- **THEN** one row `(x=a, y=a, z=a, r1=e1, r2=e1)` is returned

#### Scenario: Parallel self-loops
- **WHEN** `create` made two live eids e1 and e2 for `(a knows a)` and the IR above runs under RelIsomorphism
- **THEN** exactly two rows are returned, `(r1=e1, r2=e2)` and `(r1=e2, r2=e1)`, where Homomorphism would return four

#### Scenario: Different match groups are not constrained
- **WHEN** two relationship patterns belong to different match groups under RelIsomorphism
- **THEN** they may bind the same statement in one row

### Requirement: Set versus bag semantics
Under `graph_set = BagOfEids`, every visible statement matching a TriplePattern SHALL contribute its own solution. Under `graph_set = SetOfTriples`, a TriplePattern that does not bind an eid variable SHALL match each distinct `(s, p, o)` visible in its View exactly once, however many visible eids carry it. A pattern that binds an eid variable SHALL contribute one solution per statement under both settings.

#### Scenario: Parallel edges
- **WHEN** `create` made two live eids for `(alice called bob)` and `TriplePattern(v:alice, v:called, ?x)` is executed
- **THEN** SetOfTriples returns one row and BagOfEids returns two rows

#### Scenario: Episodes under set semantics
- **WHEN** two live statements carry `(alice worksAt acme)` with disjoint valid intervals and the pattern is executed under `{Now, Unfiltered}` with SetOfTriples
- **THEN** one row is returned

#### Scenario: History under set semantics
- **WHEN** `(alice worksAt acme)` was asserted, retracted and asserted again, giving two eids, and the pattern runs under History with SetOfTriples
- **THEN** one row is returned, and binding the eid variable instead returns two rows

#### Scenario: Count under set semantics
- **WHEN** an IR `Aggregate(group = [], count(*) as ?n)` over `TriplePattern(v:alice, v:called, ?x)` runs under SetOfTriples with the two parallel edges above
- **THEN** `?n = 1`

### Requirement: Missing values, unbound versus NULL
A missing value SHALL be returned as an absent cell under both settings of `missing`. Under `missing = Unbound`, a variable that may be missing on one side of a Join or LeftJoin SHALL be compatible with any value on the other side, and the joined row SHALL take the bound value. Under `missing = Null3VL`, a missing value SHALL never equal anything, so a join on it produces no row.

#### Scenario: Join after optional, unbound semantics
- **WHEN** an IR `Join[LeftJoin(TriplePattern(?p, v:name, ?n), TriplePattern(?p, v:email, ?e)), TriplePattern(?q, v:contact, ?e)]` runs under Unbound, and alice has no email
- **THEN** alice's row joins every `(?q contact ?e)` row, with `?e` taken from the right side

#### Scenario: Join after optional, null semantics
- **WHEN** the same IR runs under Null3VL
- **THEN** alice's row joins nothing and is not returned

#### Scenario: Absent cell in results
- **WHEN** a row has no binding for a projected variable
- **THEN** the result cell for that column is absent (no value), under both settings

### Requirement: Filter truth values
Filter and LeftJoin conditions SHALL use three-valued logic in which errors and missing values count as unknown. A row SHALL be kept only when the condition is true. A comparison involving a missing value, and an ordering comparison between values of incompatible kinds, SHALL be unknown. `NOT` of unknown SHALL be unknown, `unknown OR true` SHALL be true, and `unknown AND false` SHALL be false. An "is bound / is not null" test SHALL be true or false, never unknown.

#### Scenario: Comparison with a missing value
- **WHEN** a Filter `?age > 30` is applied to a row where `?age` is missing
- **THEN** the row is removed, and `NOT(?age > 30)` removes it too

#### Scenario: Disjunction rescues unknown
- **WHEN** a Filter `?age > 30 OR ?n = "bob"` is applied to bob's row where `?age` is missing
- **THEN** the row is kept

#### Scenario: Incompatible ordering comparison
- **WHEN** a Filter `?x < 5` is applied to a row where `?x` is the string `"abc"`
- **THEN** the row is removed

#### Scenario: Bound test
- **WHEN** a Filter `NOT bound(?e)` is applied after an optional pattern
- **THEN** exactly the rows where `?e` is missing are kept

### Requirement: Value equality and comparison
Equality between two terms SHALL be true when they are the same term. Numeric terms (integer, double, decimal) SHALL compare by numeric value across their kinds. Terms of different non-numeric kinds SHALL be unequal (false, not unknown). Ordering comparisons SHALL compare numbers numerically, strings by Unicode code point, and dates and datetimes chronologically. Datetimes SHALL compare by instant: two datetimes that denote the same instant with different timezone offsets SHALL be equal by value, while remaining distinct terms under `sameTerm`, in shared-variable joins and in `distinct`. A datetime without a timezone SHALL compare as if it were UTC. A constant compared by value SHALL work even when it is missing from the term dictionary.

#### Scenario: Integer equals decimal
- **WHEN** a Filter `?x = 1.0` (decimal) is applied to a row where `?x` is the integer `1`
- **THEN** the row is kept

#### Scenario: String is not an IRI
- **WHEN** a Filter `?x = "http://ex/a"` is applied to a row where `?x` is the IRI `<http://ex/a>`
- **THEN** the row is removed

#### Scenario: Range over long strings
- **WHEN** a Filter `?name < "Mzzzzzzzzz"` is applied, the constant is not in the dictionary, and names include `"Alexander"` (dictionary) and `"Zoe"` (inline)
- **THEN** `"Alexander"` is kept and `"Zoe"` is removed

#### Scenario: Double range
- **WHEN** a Filter `?score >= 0.5` is applied to doubles 0.25, 0.5 and 2.0
- **THEN** 0.5 and 2.0 are kept

#### Scenario: Same instant, different offsets
- **WHEN** `?a` is the datetime `"2026-03-01T12:00:00+02:00"` and `?b` is `"2026-03-01T10:00:00Z"`
- **THEN** a Filter `?a = ?b` keeps the row, and `?a < ?b` and `?a > ?b` remove it
- **AND** a Filter `sameTerm(?a, ?b)` removes the row
- **AND** `Project{distinct}` over rows binding each of them returns two rows, each decoding to its own lexical offset

#### Scenario: Datetime constant matches any offset
- **WHEN** a Filter `?t = "2026-03-01T12:00:00+02:00"^^xsd:dateTime` is applied to `TriplePattern(?e, v:at, ?t)` and one stored object is `"2026-03-01T10:00:00Z"`
- **THEN** that row is kept, and the explained plan still seeks an index on the object instead of scanning

### Requirement: Ordering by decoded value
OrderLimit SHALL sort by the decoded values of its keys, never by raw ObjectId. Within a kind: numbers (integer, double and decimal together) numerically; strings by Unicode code point, then language tag; booleans false before true; dates and datetimes chronologically. Across kinds, the order SHALL be fixed: blank and anonymous nodes, IRIs, statements, transactions, then literals (numbers, booleans, datetimes, dates, strings, language strings, other typed literals). Missing values SHALL sort first in ascending order under `missing = Unbound`, and last in ascending order under `missing = Null3VL`. Descending order SHALL reverse the value order. Ties SHALL keep a deterministic order for a given database state. Result rows SHALL follow the order of an OrderLimit at the root, or directly under a root Project or Extend.

#### Scenario: Strings in and out of the dictionary
- **WHEN** rows with `?n` = `"Zoe"` (inline), `"Alexander"` (dictionary) and `"Bo"` (inline) are ordered ascending by `?n`
- **THEN** the order is `"Alexander"`, `"Bo"`, `"Zoe"`

#### Scenario: Doubles
- **WHEN** doubles 10.5, -2.0 and 3.25 are ordered ascending
- **THEN** the order is -2.0, 3.25, 10.5

#### Scenario: Mixed integers and doubles
- **WHEN** the values integer 3, double 2.5 and integer -7 are ordered ascending
- **THEN** the order is -7, 2.5, 3

#### Scenario: Negative datetimes
- **WHEN** datetimes before and after 1970 are ordered ascending
- **THEN** they come out chronologically

#### Scenario: Missing values placement
- **WHEN** rows where `?age` is 30, missing and 20 are ordered ascending
- **THEN** under Unbound the order is missing, 20, 30, and under Null3VL it is 20, 30, missing

#### Scenario: Descending
- **WHEN** the strings `"a"`, `"b"` and `"c"` are ordered descending
- **THEN** the order is `"c"`, `"b"`, `"a"`

### Requirement: Aggregates
Aggregate SHALL support `count` (of rows or of bound values, optionally distinct), `sum`, `avg`, `min`, `max`, `sample`, `group_concat` (with separator) and `collect`. `sum` and `avg` SHALL use numeric values. `min` and `max` SHALL use the value order of OrderLimit. Missing values SHALL be ignored by every aggregate except `count(*)`. `collect` SHALL return a list of the bound values. Without grouping keys, an empty input SHALL produce one row, where `count` is 0, `collect` is an empty list, and the other aggregates are missing.

#### Scenario: Min over strings uses value order
- **WHEN** `min(?n)` is computed over `"Zoe"`, `"Alexander"` and `"Bo"`
- **THEN** the result is `"Alexander"`

#### Scenario: Average of integers
- **WHEN** `avg(?age)` is computed over 20, 30 and a missing value
- **THEN** the result is 25 and the missing value is ignored

#### Scenario: Count distinct
- **WHEN** `count(distinct ?c)` is computed over acme, acme and globex
- **THEN** the result is 2

#### Scenario: Collect
- **WHEN** `collect(?c)` is grouped by person
- **THEN** each person's row has a list of that person's companies

#### Scenario: Grouping with missing keys
- **WHEN** rows are grouped by `?e` and some have `?e` missing
- **THEN** all rows with a missing `?e` form one group

### Requirement: Typed result decoding
Every result cell SHALL be decoded to a typed value: IRIs to IRI values; anonymous and blank nodes to node values; statements to statement (eid) values; transactions to transaction values; integers, booleans, dates, datetimes, strings, language-tagged strings, doubles, decimals and other typed literals to the matching literal values; aggregate and expression results to literal values of their computed type; and `collect` results to lists. Decoding SHALL give the same values whether terms come from the term cache or from the dictionary.

#### Scenario: Every ObjectId kind decodes
- **WHEN** a query returns one value of each ObjectId tag
- **THEN** each cell holds the value that was stored, of the matching kind, and datetimes come back as their epoch-millisecond instant together with their original timezone offset, or no offset when none was given

#### Scenario: Typed literal with its datatype
- **WHEN** a query returns a `TYPED` literal `"P3D"^^xsd:duration`
- **THEN** the cell holds lexical form `"P3D"` with datatype IRI `xsd:duration`

#### Scenario: Computed values
- **WHEN** a query returns `count(*)` and `avg(?score)` over doubles
- **THEN** the cells hold an integer literal and a double literal

#### Scenario: Cold and warm cache agree
- **WHEN** the same query is executed on a fresh database handle and again on a warmed one
- **THEN** both results are equal

### Requirement: Bounded term cache
Decoding SHALL go through a bounded least-recently-used cache from ObjectId to term, shared by all readers of one database handle, with a configurable capacity. Because terms are immutable and never deleted, cached entries SHALL stay valid across transactions. Entries learned on the writer connection inside a speculative `with` SHALL NOT be inserted into the shared cache. Each query result SHALL report how many term lookups were served by the cache and how many read the dictionary.

#### Scenario: Warm cache avoids dictionary reads
- **WHEN** the same query that returns 100 distinct dictionary terms is executed twice, with a capacity of at least 100
- **THEN** the second result reports 0 dictionary reads

#### Scenario: Capacity is respected
- **WHEN** the capacity is 10 and a query decodes 50 distinct dictionary terms
- **THEN** the cache holds at most 10 entries afterwards, and all 50 cells decode correctly

#### Scenario: Speculation does not pollute the cache
- **WHEN** a query inside `db.with` returns a term that the speculative transaction created, and the speculation is rolled back
- **THEN** the shared cache holds no entry for that term's id afterwards

### Requirement: Connection selection
Outside a speculative `with`, a query SHALL run on a connection from the reader pool, inside one read transaction that covers planning lookups, execution and decoding, and SHALL release the connection when done. Inside `db.with(ops, |view| …)`, queries on the speculative view SHALL run on the writer connection within the open savepoint, so that they see the uncommitted speculative state. Query execution SHALL never write to the database on either connection.

#### Scenario: Reader for ordinary queries
- **WHEN** a query runs while another thread holds the writer inside a long transaction
- **THEN** the query completes without waiting for the writer, and sees the last committed state

#### Scenario: Speculative view sees uncommitted state
- **WHEN** `db.with(|tx| tx.assert(alice, likes, tea), |view| view.execute_ir(q))` runs a query for `(alice likes ?x)`
- **THEN** the result contains `tea`
- **AND** after `with` returns, the same query on `db.now()` does not contain `tea`

#### Scenario: Speculative constant encoding
- **WHEN** a speculative transaction creates the IRI `v:newThing` and a speculative query uses that IRI as a constant
- **THEN** the pattern matches the speculative statement and is not short-circuited
- **AND** after rollback, the same query on `db.now()` short-circuits to empty

### Requirement: Native operator extension point
The planner SHALL split the IR into regions and route each PathPattern to a registered native path operator, which SQL reaches as the table-valued function `tm_path(start, path, mode, max_hops, view)` returning `(start, end, hops, path_json)`. The call SHALL receive the pattern's View in its `view` argument, and a start value that is a bound constant, a parameter or a column of an earlier FROM item. When no path operator is registered, a query containing a PathPattern SHALL fail with `Unsupported` before any SQL is executed. Cyclic basic graph patterns SHALL be detected and routed to SQL while the LFTJ extension point is disabled, which is the only setting in this change.

#### Scenario: No path operator registered
- **WHEN** an IR containing a PathPattern is executed on a database with no path operator registered
- **THEN** execution fails with `Unsupported` naming path patterns, and no SQL is executed

#### Scenario: Path composes as a table-valued function
- **WHEN** a test path operator is registered and an IR joins `TriplePattern(?b, v:supportedBy, ?r)` with `PathPattern(?r, ?x, sys:subject*)`
- **THEN** the explained SQL contains a single statement with `tm_path(…)` as a FROM item whose start argument is the column bound to `?r`
- **AND** the view argument encodes the path pattern's View

#### Scenario: End-bound path
- **WHEN** a PathPattern has an unbound start and a constant end
- **THEN** the planner invokes the path operator from the bound end with the inverse path, and binds the start variable from its `end` column

#### Scenario: Triangle query without LFTJ
- **WHEN** a cyclic pattern `(?a knows ?b), (?b knows ?c), (?c knows ?a)` is executed
- **THEN** the explain output reports the region as cyclic and routed to SQL, and the result equals a brute-force enumeration

### Requirement: Explain
The facade SHALL provide an explain operation for an IR and its parameters that returns, without running the query: the region routing, whether the query short-circuited, the SQL text, the ordered list of bound parameter values, and SQLite's `EXPLAIN QUERY PLAN` rows, both for the whole statement and for each SQL region. The plan SHALL be taken with the actual parameter values bound. Explaining a short-circuited query SHALL return no SQL.

#### Scenario: Explain a normal query
- **WHEN** a single-pattern Now query on a churned predicate is explained
- **THEN** the output has one SQL region, the SQL text, its parameters, and a query plan naming a `live_*` index

#### Scenario: Plan per region
- **WHEN** a query that joins a two-pattern BGP with a path pattern is explained with a test path operator registered
- **THEN** the SQL region carries its own `EXPLAIN QUERY PLAN` rows, which name the scans of its own `triple` aliases in plan order

#### Scenario: Explain a short-circuited query
- **WHEN** a query whose constant is missing from the dictionary is explained
- **THEN** the output reports a short-circuit and has no SQL text

### Requirement: Join order from statistics
The executor SHALL leave the join order of a SQL region to SQLite's planner, and plan quality SHALL NOT depend on the textual order of the patterns in the IR, given the planner statistics that the store keeps current. The executor SHALL NOT require an explicit `optimize()` call for good plans. Stale or missing statistics SHALL only change a plan's speed, never its results. An engine-forced join order is not part of this requirement.

The skewed fixture used below is loaded through the ordinary API with bound parameters: one class holds 90 % of the nodes, one predicate has 50 rows, and some properties are churned (asserted, retracted and asserted again).

#### Scenario: Selective pattern first
- **WHEN** a four-pattern BGP on the skewed fixture joins the 90 % class, a high-fanout predicate and the 50-row predicate, and is explained without an explicit `optimize()` call
- **THEN** the region's query plan starts from the 50-row predicate's pattern

#### Scenario: Textual order does not matter
- **WHEN** the same BGP is explained with its patterns permuted in the IR
- **THEN** every permutation's plan starts from the same most selective pattern, and all permutations return the same result multiset

#### Scenario: Predicate-only patterns
- **WHEN** a BGP whose patterns bind only their predicates, one of them the 50-row predicate, is explained on the skewed fixture
- **THEN** the plan starts from the 50-row predicate's pattern

#### Scenario: Stale statistics keep results
- **WHEN** a large batch of statements is committed after the statistics were gathered, and a golden BGP runs before any further `PRAGMA optimize`
- **THEN** its result equals the result of the same BGP after `Db::optimize()`

### Requirement: Host capabilities
The query engine SHALL reach SQLite only through the store's executor boundary, and SHALL require the host capabilities `functions` (its SQL helper functions) and `vtab` (the `tm_path` and `rarray` virtual tables). Opening a database with the query engine on a host that lacks either capability SHALL fail with a clear typed error that names the missing capability, and SHALL NOT fall back to a degraded mode. With both capabilities present, the helper functions and native operators SHALL be registered through the host on the writer and on every pooled reader.

#### Scenario: Host without virtual tables
- **WHEN** a database is opened with the query engine on a test host that declares `functions` but not `vtab`
- **THEN** opening fails with an error naming the `vtab` capability, and no query is ever planned

#### Scenario: Host without functions
- **WHEN** a database is opened with the query engine on a test host that declares `vtab` but not `functions`
- **THEN** opening fails with an error naming the `functions` capability

#### Scenario: First host has both
- **WHEN** a database is opened on the `rusqlite` host and a query using a value comparison runs on a pooled reader and inside `db.with`
- **THEN** both succeed, because the helper functions are registered on every connection

### Requirement: Typed errors
Execution failures SHALL be reported as typed errors: `InvalidQuery` for structurally invalid IR and missing parameters, `Unsupported` for features not available in this build (path patterns without an operator, a path with no bound endpoint), a storage error that carries SQLite's message for failures inside SQLite, and, at open time, an error naming a missing host capability (see Host capabilities). A failed query SHALL leave no state behind: the connection goes back to the pool and the shared cache holds only correct entries.

#### Scenario: Error returns the connection
- **WHEN** a query fails with `Unsupported` on a pool with one reader
- **THEN** a following valid query succeeds on that same pool

### Requirement: Duplicate removal only for multi-eid predicates
Under `graph_set = SetOfTriples`, the executor SHALL skip duplicate removal for a TriplePattern whose predicate is a constant absent from `pred_multi`. It SHALL keep duplicate removal for every other pattern that does not bind an eid. Results SHALL be the same as with duplicate removal applied everywhere, in every view.

#### Scenario: Predicate without duplicates skips removal
- **WHEN** `v:name` has never held two eids with the same `(s, p, o)` and a SetOfTriples query matches `?x v:name ?n`
- **THEN** the generated SQL for that pattern contains no canonical-eid subquery

#### Scenario: A parallel edge turns removal on
- **WHEN** `create` adds a second `(v:alice v:called v:bob)` and the same SetOfTriples query ran before and runs again after
- **THEN** after the write, `v:called` is in `pred_multi`, the SQL contains the canonical-eid subquery, and the query still returns one row

#### Scenario: Re-assert after retract under History
- **WHEN** `(v:alice v:worksAt v:acme)` is asserted, retracted and asserted again, and a SetOfTriples pattern on `v:worksAt` runs under History
- **THEN** it returns one row for that triple
