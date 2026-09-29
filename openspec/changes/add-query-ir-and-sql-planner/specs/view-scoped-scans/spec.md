## Purpose

Defines how a pattern's View (transaction-time and valid-time selectors) becomes predicates on the triple table. There is exactly one mapping, it always produces the verified index-friendly shapes, and historical answers never change once they are computable.

## ADDED Requirements

### Requirement: Single view-to-predicate mapping
Every scan of the triple table in generated SQL SHALL carry exactly the predicates that its pattern's View maps to, and no other time predicates. This includes pattern aliases, correlated subqueries used for set semantics, virtual-predicate lookups and path-operator arguments. The mapping SHALL be:

| View part | Predicate on alias `a` |
|---|---|
| `tx = Now` | `a.t_ret IS NULL` |
| `tx = AsOf(t)` | `a.t_add <= :t AND (a.t_ret IS NULL OR a.t_ret > :t)` |
| `tx = History` | none |
| `valid = Unfiltered` | none |
| `valid = At(d)` | `(a.v_from IS NULL OR a.v_from <= :d) AND (a.v_to IS NULL OR a.v_to > :d)` |

The transaction-time and valid-time parts SHALL combine with `AND`.

#### Scenario: Now, unfiltered
- **WHEN** the SQL for `TriplePattern(v:alice, v:worksAt, ?c)` under View `{Now, Unfiltered}` is explained
- **THEN** the only time predicate on its alias is `t_ret IS NULL`

#### Scenario: AsOf, unfiltered
- **WHEN** the SQL for the same pattern under `{AsOf(Tx(150)), Unfiltered}` is explained
- **THEN** its alias carries `t_add <= ? AND (t_ret IS NULL OR t_ret > ?)`, and both placeholders are bound to 150

#### Scenario: History, unfiltered
- **WHEN** the SQL for the same pattern under `{History, Unfiltered}` is explained
- **THEN** its alias carries no predicate on `t_add`, `t_ret`, `v_from` or `v_to`

#### Scenario: Valid-at combined with each transaction selector
- **WHEN** the pattern is explained under `{Now, At(d)}`, `{AsOf(Tx(150)), At(d)}` and `{History, At(d)}`
- **THEN** each alias carries the valid-time predicate with `d` bound, ANDed with that selector's transaction-time predicate (none for History)

### Requirement: Verbatim live predicate
Under `tx = Now` the generated SQL SHALL contain the predicate `t_ret IS NULL` verbatim on the pattern's alias, so that SQLite can choose the partial `live_*` and `valid_p` indexes. The executor SHALL NOT express it in any other form (for example `t_ret IS ?`, `coalesce(t_ret, …)` or `NOT (t_ret IS NOT NULL)`). For a pattern on the optional side of a LeftJoin, its view predicates SHALL be placed in that join's ON clause.

#### Scenario: Live predicate text
- **WHEN** the SQL of any IR containing a Now pattern with alias `t0` is explained
- **THEN** the SQL text contains `t0.t_ret IS NULL`

#### Scenario: Optional pattern keeps its view in ON
- **WHEN** an IR `LeftJoin(TriplePattern(?p, v:name, ?n), TriplePattern(?p, v:email, ?e))` under Now is explained
- **THEN** the optional pattern's `t_ret IS NULL` appears in the `LEFT JOIN … ON` clause and not in `WHERE`
- **AND** people without a live email are returned with `?e` missing

### Requirement: Transaction-time visibility
A statement SHALL be visible under `Now` if and only if it is live (`t_ret` is NULL). It SHALL be visible under `AsOf(t)` if and only if `t_add ≤ t` and (`t_ret` is NULL or `t_ret > t`). It SHALL be visible under `History` always, whether live or retracted.

#### Scenario: Retracted at t
- **WHEN** statement e was added at tx 5 and retracted at tx 9
- **THEN** e is visible under `AsOf(Tx(5))` and `AsOf(Tx(8))`, not under `AsOf(Tx(9))`, `AsOf(Tx(4))` or `Now`, and visible under `History`

#### Scenario: Added at t
- **WHEN** statement e was added at tx 7 and is live
- **THEN** e is visible under `AsOf(Tx(7))` and `Now`, and not under `AsOf(Tx(6))`

#### Scenario: Cascaded rows follow the same rule
- **WHEN** retracting e1 at tx 9 cascaded to its annotation e2
- **THEN** a query joining e1 and its annotation under `AsOf(Tx(8))` returns the pair, and under `Now` returns nothing

### Requirement: Valid-time visibility
Under `valid = At(d)`, a statement SHALL be visible if and only if its half-open interval `[v_from, v_to)` contains `d`, where a NULL bound is unbounded. Under `valid = Unfiltered`, valid time SHALL NOT filter at all, so the default never hides past or future facts.

#### Scenario: Half-open upper bound
- **WHEN** a statement is valid `[2025-01-01, 2026-03-01)`
- **THEN** it is visible under `At(2026-02-28T23:59:59.999Z)` and not under `At(2026-03-01T00:00:00Z)`

#### Scenario: Inclusive lower bound
- **WHEN** a statement has `v_from = d`
- **THEN** it is visible under `At(d)` and not under `At(d − 1 ms)`

#### Scenario: Unbounded statement
- **WHEN** a statement has both `v_from` and `v_to` NULL
- **THEN** it is visible under every `At(d)`

#### Scenario: Unfiltered includes ended facts
- **WHEN** a live statement's valid interval ended last year and a pattern uses `valid = Unfiltered`
- **THEN** the statement is returned

#### Scenario: Episodes under valid-at
- **WHEN** two live statements carry the same `(alice worksAt acme)` with intervals 2020–2022 and 2024–open
- **THEN** `At(2021-06-01)` matches only the first, `At(2023-06-01)` matches neither, and `At(2025-06-01)` matches only the second

### Requirement: Mixed views in one plan
One generated SQL statement SHALL be able to contain patterns under different Views. Each alias SHALL carry only its own pattern's predicates, and the time values SHALL be bound as separate parameters.

#### Scenario: Before and after a supersede
- **WHEN** e1 `(alice worksAt acme)` is superseded at tx 200 into `(alice worksAt globex)`, and an IR joins `TriplePattern(v:alice, v:worksAt, ?before)` under `AsOf(Tx(150))` with `TriplePattern(v:alice, v:worksAt, ?after)` under `Now`, filtered by `?before != ?after`
- **THEN** the result is exactly one row, `?before = acme`, `?after = globex`
- **AND** the explained SQL has one alias carrying the AsOf predicate and another carrying `t_ret IS NULL`

#### Scenario: Three views in one query
- **WHEN** an IR joins patterns under `Now`, `AsOf(Tx(3))` and `{History, At(d)}`
- **THEN** each alias's predicates match its own view, and no alias carries another alias's predicates

### Requirement: Index-friendly scans
For a single pattern with at least one bound position among subject, predicate and object, the SQLite query plan SHALL use an index of the view's family and SHALL NOT fully scan `triple`. A Now pattern with no valid-time filter SHALL use a covering `live_*` index: `live_spo` when the subject is bound, `live_pos` when only the predicate (with or without the object) is bound, and `live_osp` when only the object is bound. An `AsOf` or `History` pattern SHALL use a covering `hist_*` index chosen by the same rule. A Now pattern with a valid-time filter and a bound predicate SHALL use `live_pos` or `valid_p`. A pattern whose eid equals a constant SHALL use the integer primary key.

#### Scenario: Now, subject and predicate bound
- **WHEN** `EXPLAIN QUERY PLAN` is taken for `TriplePattern(v:alice, v:worksAt, ?o)` under Now
- **THEN** the plan contains `USING COVERING INDEX live_spo`

#### Scenario: AsOf, subject and predicate bound
- **WHEN** it is taken for the same pattern under `AsOf(Tx(150))`
- **THEN** the plan contains `USING COVERING INDEX hist_spo`

#### Scenario: Object bound
- **WHEN** it is taken for `TriplePattern(?s, ?p, v:acme)` under Now
- **THEN** the plan contains `USING COVERING INDEX live_osp`

#### Scenario: Predicate bound under History
- **WHEN** it is taken for `TriplePattern(?s, v:worksAt, ?o)` under History
- **THEN** the plan contains `USING COVERING INDEX hist_pos`

#### Scenario: Valid-at over current beliefs
- **WHEN** it is taken for `TriplePattern(?s, v:worksAt, ?o)` under `{Now, At(d)}`
- **THEN** the plan uses `live_pos` or `valid_p`, and not a full scan of `triple`

#### Scenario: Eid lookup
- **WHEN** it is taken for a pattern whose eid variable is filtered to equal a constant statement id
- **THEN** the plan contains `USING INTEGER PRIMARY KEY`

### Requirement: Time values are bound parameters
The transaction number of `AsOf` and the instant of `At` SHALL be bound as SQL parameters and SHALL NOT appear in the SQL text. Two IRs that differ only in their time values SHALL generate byte-identical SQL text.

#### Scenario: Same text for different t
- **WHEN** the same IR is explained under `AsOf(Tx(10))` and under `AsOf(Tx(99))`
- **THEN** both SQL texts are identical and only the bound parameter values differ

### Requirement: Stable historical results
For every `t` up to the last committed transaction, an IR whose patterns all use `AsOf(Tx(t))` SHALL return the same result multiset whenever it is executed, regardless of later transactions, including retracts, supersedes, cascades and cardinality-one replacements.

#### Scenario: Recompute after later writes
- **WHEN** an AsOf(Tx(t)) join query over statements and their annotations is executed, then transactions retract, supersede and cascade those statements, and the query is executed again
- **THEN** both executions return identical result multisets

#### Scenario: Property test over random histories
- **WHEN** random operation sequences are applied and, for each committed t, an AsOf(Tx(t)) query result is recorded
- **THEN** re-running every recorded query after all operations returns the recorded result

### Requirement: One snapshot per query
All reads of one query execution SHALL observe a single database snapshot: resolving time references, looking up constants, running the SQL, and decoding result terms. A transaction committed concurrently SHALL be either entirely visible or entirely invisible to that execution.

#### Scenario: Concurrent commit during a query
- **WHEN** a writer commits a transaction that retracts e1 and asserts e2 while a Now query on a reader is between planning and decoding
- **THEN** the query's result reflects either the state before that transaction or the state after it, never a mix
