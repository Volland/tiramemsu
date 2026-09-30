---
lat:
  require-code-mention: true
---
# Tests

Test specifications for the invariants the design depends on. Each leaf is one test or one property test, and implementation code references it with an `@lat:` comment.

Every leaf is referenced by exactly one `@lat:` comment in test code, and `lat check` enforces it (`require-code-mention`). The Query leaves belong to the query engine crates (`tm-exec`, `tm-sparql`, `tm-cypher`) and the facade.

## Time Travel

Properties of transaction time. They are checked with property-based tests (`proptest`) over random operation sequences.

### AsOf Equals Replay

For random sequences of assert, create, retract, supersede and cardinality-one operations: for every t, `asOf(t)` must equal the state rebuilt by replaying `event` rows with time ≤ t.

### Instants Are Monotonic

Committing transactions while the mocked clock moves backwards still gives strictly increasing `tx.instant`, and `asOf(instant)` resolves to the correct `t`.

### Historical Reads Are Stable

A result of `asOf(t)` computed before later transactions (retracts, supersedes, cascades) is identical when recomputed afterwards.

## Operations

Behaviour of the write operations in [[time-model#Operations]].

### Assert Is Idempotent

Asserting the same `(s, p, o)` twice with overlapping valid time returns `Existing` with the first eid, and adds no row.

### Non-Overlapping Episodes Coexist

Asserting the same `(s, p, o)` with disjoint valid intervals creates two live eids.

### Create Makes Parallel Edges

`create` of an identical live triple always returns a new eid, and both are live.

### Retract Is Once

Retracting a retracted eid returns false and changes nothing; `t_ret` keeps its first value.

### Supersede Replays Layers

Superseding e1 (with annotation e2 and reference e7) creates new eids for all three, rewires `s`/`o` through σ, keeps annotation content, links the roots with `sys:supersedes`, and marks the old eids `ret_kind = supersede`.

### Cardinality One Does Not Replay

On a `sys:one` predicate, asserting a new object retracts the old statement and its annotations (`ret_kind = cardinality`), and does not copy the annotations to the new statement.

### Unique Rejects Second Subject

With `sys:unique` on p, asserting `(s2, p, o)` while `(s1, p, o)` is live fails with `UniqueViolation`, and the transaction leaves no trace.

### Upsert Returns Existing Node

`upsert(p, o)` on a unique predicate returns the existing subject when there is one, and otherwise creates a new `NODE`.

## Cascade

Behaviour of [[time-model#Cascade]].

### Cascades Subject And Object

Retracting e1 retracts, in the same tx, live statements with `s = e1` and with `o = e1`, recursively, and leaves triples about the plain nodes untouched.

### Cascade Terminates On Cycles

Two statements that reference each other are both retracted exactly once.

### Cascade Limit Aborts

A cascade larger than `max_cascade` fails with `CascadeLimitExceeded`, and the database is unchanged.

### Dry Run Reports Without Commit

`dry_run` returns the full retracted set while the triple and tx tables stay unchanged. Only the burned-id counters in `meta` advance.

## Storage Invariants

Checks of the SQLite-level guarantees in [[storage#Invariant Triggers]] and [[storage#Query Shapes]].

### Triggers Block Deletion

DELETE on `triple`, `term` or `tx`, a second retraction, and any change to statement content are rejected by the database even through a raw SQLite connection.

### Views Use Covering Indexes

`EXPLAIN QUERY PLAN` for the generated "now", "asOf", "validAt" and object-bound shapes names a covering `live_*`, `hist_*` or `valid_p` index.

### Speculation Leaves No Trace

After `db.with`, the triple, term and tx tables are unchanged. The `meta` id counters have advanced past every id allocated during speculation, so those ids are never reissued.

### Failed Transactions Leave No Trace

A transaction body that writes statements, terms and volatile values and then fails leaves `meta`, `term`, `tx`, `triple` and `volatile` unchanged on every host, and the next commit gets the next gap-free `t`.

### Engine Never Fires A Trigger

Random sequences of every write operation never make an invariant trigger abort, and the dictionary never holds two terms with the same `(tag, lex, dt, lang)`.

### Core Runs On A Minimal Host

The whole `tm-core` suite also runs on a host that declares no capability, and no SQL the core issues calls a user function, a virtual table or FTS5. See [[architecture#Executor]].

## ObjectId

Checks of the codec in [[data-model#ObjectId]].

### Canonical Round Trip

For every tag, encode then decode returns the value. A value that can be inlined never gets a dictionary id.

### Order Within Tag

For INT, DATE and DATETIME, integer order of the ObjectIds equals value order, negative values included, under SQLite's signed comparison. For DATETIME, value order is the order of `id >> 15`.

### DateTime Keeps Its Offset

`"2026-03-01T12:00:00+02:00"` and `"2026-03-01T10:00:00Z"` get two ObjectIds that decode to their own lexical offsets. SPARQL `=` and Cypher `=` on them are true, and `sameTerm` is false.

### Reserved Tag Is Rejected

Format 1 rejects tag 15 `SEALED` in stored values and the `sys:sensitive` schema flag, until M6. See [[time-model#Erasure]].

## Query

Checks of [[query#Front Ends]] and [[query#Physical Planning#Path Engine]].

### Differential SPARQL Cypher

A corpus of equivalent SPARQL/Cypher query pairs over shared fixtures returns identical result multisets, after normalising for set-versus-bag semantics.

### openCypher TCK

The openCypher TCK (2024.3) runs end to end on `tm-cypher`. Scenarios expected to fail are listed with a reason in `crates/tm-cypher/tests/tck/allowlist.txt`, and the run fails on an unexpected failure and on an unexpected pass.

### Dual View Binds Same Eid

A Cypher relationship variable used in node position and a SPARQL `~ ?r` reifier bind the same eid for the same fact.

### Per Pattern Time Scopes

A query that mixes an `asOf(t)` pattern and a now pattern returns the before and after values of a superseded fact. In SPARQL, the `asOf` pattern sits in `SERVICE <tm:asOf/t>`, and a `tm:` IRI in `GRAPH` fails with a `Parse` error.

### SPARQL Golden Cases

Every case under `crates/tm-sparql/tests/golden` runs on a fresh database and is compared with the files next to the query: IR text, SPARQL JSON or N-Triples, or the error. A rejected request runs no SQL and opens no transaction.

### SPARQL W3C Subset

The manifest-driven runner executes the in-scope SPARQL 1.0, 1.1 and 1.2 categories on fresh databases, with `graphData` as named graphs. It fails on any failure not in `expected-deviations.toml`, and on any listed test that now passes.

### Skewed Joins Start Selective

With bound parameters on a skewed fixture, every golden BGP's plan starts from its most selective pattern, without an explicit `optimize()` call.

The fixture has one class holding 90 % of the nodes, a 50-row predicate and churned properties. It is loaded through the ordinary API, so the test also checks that statistics appear automatically. See [[query#Physical Planning#Join Ordering]].

### Unknown Constant Short Circuits

A pattern with a constant that is missing from the term dictionary empties its join, and the query runs no SQL statement and inserts no term. See [[query#Logical IR]].

### Set Semantics Dedupes Eids

Under `SetOfTriples`, parallel eids for one `(s, p, o)` match once unless the eid is bound, and duplicate removal is skipped for a predicate absent from `pred_multi`. See [[storage#Multi-Eid Predicates]].

### Isomorphism Excludes Reused Eids

Under `RelIsomorphism`, two relationship patterns of one match group never bind the same eid in a row, while `Homomorphism` and different match groups allow it. See [[query#Physical Planning#SQL Codegen]].

### Order By Decoded Value

`ORDER BY` sorts by decoded value across numeric kinds, dictionary and inline strings, and datetimes, with missing values first under `Unbound` and last under `Null3VL`. See [[data-model#ObjectId#Range Scans]].

### Paths Cross Layers

A path using `sys:subject` hops finds the entities behind a statement that a belief references.

### Unbounded Paths Are Capped

A Cypher `-[*]->` pattern without an upper bound stops at `max_hops` and does not fail.

### Statement Instants Come From The Tx Table

`tm:addedAt` and `tm:retractedAt` return the commit instants of `t_add` and `t_ret` as UTC date-times, under Now, History and `asOf` (which shows a later retraction), with one triple alias for a bound eid.

A constant with another offset compares by instant and seeks `tx_instant`, and an instant of no transaction or a constant of another kind matches nothing. See [[query#Views and Scans#Virtual Predicates]].

### Statement Instants Filter By Instant

A `FILTER` comparing `tm:addedAt` with a date-time constant in another offset keeps exactly the statements committed after or before that instant.

### Bitemporal Recipes In SPARQL

With a manual clock, "learned late" (`addedAt > validFrom`) and "recorded after it stopped being true" (`addedAt > validTo`) return the expected statements, and a constant instant finds every statement of that transaction.

### Cypher Statement Instants

`r.addedAt` and `r.retractedAt` read the commit instants on a relationship, its node form and in `USE HISTORY`, and a property map matches them.

`keys()` omits them, a stored `v:addedAt` stays reachable by CURIE, and `SET` or `REMOVE` of them is `Unsupported` and writes nothing.

### Bitemporal Recipes In Cypher

`r.addedAt.epochMillis - r.validFrom.epochMillis > $days * 86400000` finds statements learned more than N days late, and `r.addedAt > r.validTo` finds statements recorded after they stopped being true.

## Named Graphs

Checks of [[data-model#Named Graphs]] across the core, the planner, SPARQL query and update, and Cypher.

### Membership Is Engine Owned

Asserting `sys:inGraph` through `Tx::assert` or `create` fails with `ReservedNamespace`, and a schema statement cannot be added to a graph. Nothing is written in either case. See [[data-model#Named Graphs]].

### Graph Names Are Validated

A literal or a statement as graph name fails with `InvalidGraphName` on every graph method and writes nothing, while IRI, `NODE` and `BNODE` names work.

### Error Messages Name The Term

`InvalidGraphName`, `GraphNotFound` and `GraphExists` render messages that name the rejected term or graph.

### Statement Can Be In Many Graphs

One statement added to two graphs keeps one eid and gets two memberships, adding twice is idempotent, and a retracted statement cannot be added. The report lists memberships apart from statements.

### Remove And Clear Keep Statements

`remove_from_graph` and `clear_graph` retract memberships only, removing a non-member is a no-op, `create_graph` is idempotent, and `drop_graph` retracts the declaration and keeps other metadata.

### Views Apply To Membership

`graph_members` and `graphs` honour `asOf`, `history` and `validAt` for the membership, and the member statement must be visible in the same view.

### Cascade And Supersede Drop Memberships

Retracting a statement retracts its memberships with `ret_kind` cascade in the same transaction. Supersede and cardinality-one replacement do not copy memberships to the replacement.

### Membership Lookups Use Live Indexes

`EXPLAIN QUERY PLAN` of `(p = sys:inGraph, o = g)` and `(s = eid, p = sys:inGraph)` shows index seeks with no table scan and no new index. After churn and `ANALYZE` the graph lookup uses `live_pos`, and `sqlite_stat4` has samples for it.

### Membership Matches A Model

For random adds, removes, clears and retracts, `graph_members` of every graph equals a model of `(eid, graph)` pairs now and as of every earlier transaction.

### Graph Selector Text And Validation

The IR text form prints `:graph` for a set or a variable, the variable is in scope, and an empty set, a variable in a set or a selector on a virtual predicate is invalid.

### Graph Selector Lowers To Membership Join

A graph selector of one graph, several graphs or a variable runs as a membership join in the pattern's view. A statement in two listed graphs matches once, and `Var` gives a row per membership.

An unknown graph matches nothing, and no internal variable reaches the result.

### Graph Selector SQL Snapshots

The generated SQL for a graph set of one, of several and a variable, and for a membership under `asOf`, matches the stored `insta` snapshots.

### Small Graph Seeks First

With an 1 800-member graph and a 3-member graph over the same statements, a pattern restricted to the small graph starts its plan from the membership scan, after `ANALYZE` (D19).

### GRAPH Selects By Membership

`GRAPH <g>`, `GRAPH ?g` (one row per membership), a bound graph variable, nested `GRAPH` and an unknown graph give the rows of the spec, and a statement in no graph has no graph.

### Default Graph Is The Union

Without `FROM`, a query sees every statement once, a statement in two graphs appears once, `FILTER NOT EXISTS { ?e sys:inGraph ?g }` selects statements in no graph, and a store without graphs behaves as before.

### Dataset Clauses

`FROM` narrows the default graph to the union of the listed graphs, `FROM NAMED` limits `GRAPH` for constant and variable names, and a time IRI may sit beside a graph IRI.

### Graphs Combine With Service Scopes

`SERVICE <tm:asOf/t> { GRAPH <g> { … } }` and `GRAPH <g> { SERVICE … }` read membership and statement in the scoped view, and a `tm:` IRI as a graph name still fails with a `Parse` error that names `SERVICE`.

### Graph Metadata Is Ordinary Triples

Triples about a graph node are written and read like any triple, are visible in the default graph, and are not members of the graph.

### Membership Carries Layers

A membership is a statement: `~ ?m {| … |}` annotates it, and a statement annotated inside `GRAPH` follows the graph while its annotation triples read the default view.

### Duplicates Follow Memberships

A statement in two graphs appears once in the default graph and once per membership under `GRAPH ?g`, also for a predicate that holds parallel eids.

### Membership Is Bitemporal In SPARQL

`asOf` shows a past membership, `history` lists a removed one, `validAt` honours a bounded membership, and a plain delete retracts a statement together with its memberships.

### Property Paths Inside Graphs Are Rejected

A property path under `GRAPH` or a `FROM <g>` default graph, also inside an update `WHERE`, fails with `Unsupported("named graph path")` before any SQL runs, and the same path outside a graph still runs.

### Graph Names Are Rejected When Invalid

A statement or transaction IRI as graph name, in `GRAPH`, `FROM`, `CREATE` or a template, and a graph variable bound to a literal, fail with `InvalidGraphName` and write nothing.

### SPARQL Insert Into A Graph

`INSERT DATA` and templates into a graph assert the statement and a membership idempotently, an already-live statement only gains a membership, `GRAPH ?g` templates use the `WHERE` binding, and an unbound graph variable is a `Parse` error.

### SPARQL Delete From A Graph

Deleting in a `GRAPH` block retracts the membership only, a non-member or unknown graph is a no-op, and a plain delete retracts the statement and every membership.

### SPARQL WITH And USING

`WITH` sets the graph of templates and scopes the `WHERE`, `USING` and `USING NAMED` scope the `WHERE` default and `GRAPH`, and a time IRI as a data-block graph is a `Parse` error.

### SPARQL Graph Management

`CREATE`, `CLEAR`, `DROP` with `GRAPH` and `NAMED`, `SILENT`, `GraphNotFound` and `GraphExists` behave as specified: `CLEAR` keeps statements and `DROP` also drops the declaration but not other metadata.

### Graph Updates Fail Atomically

A failing graph operation or a rejected `LOAD`, `ADD`, `MOVE`, `COPY`, `CLEAR DEFAULT` or `DROP ALL` aborts the whole request and stores nothing.

### SPARQL Membership Predicate Is Engine Owned

`INSERT DATA` of a `sys:inGraph` triple is `ReservedNamespace`, a schema statement in a `GRAPH` block is rejected, and memberships are readable in SPARQL.

### Cardinality One Drops Old Memberships

A cardinality-one replacement written into a graph retracts the old statement and its membership with kind cardinality, and the new statement is a member.

### Cypher Keeps One Graph

`USE GRAPH g1 MATCH …` and `USE g1 …` fail with `Unsupported("USE GRAPH")` before any write, and the time forms of `USE` are unaffected.

## Recipes

The queries of [[recipes]], run on small fixtures in `crates/tiramemsu/tests/recipes.rs`. They guard combinations of features, not one feature.

### Evidence Chains Travel In Time

A path from a belief through `v:supportedBy/v:derivedFrom*` returns the retracted derivation under `SERVICE <tm:asOf/t>` and not now, and `v:supportedBy/sys:subject` reaches the subject of the supporting fact.

### Provenance From Transaction Metadata

Facts are selected by the `sys:author` of the transaction that added them, and a retracted fact is joined to the `sys:reason` of the transaction that retracted it, in the history view.

### Edit Lineage Follows Supersedes

After two supersedes, `sys:supersedes+` from the current root returns both earlier versions, from a reifier variable and from the root's skolem IRI.

### Contradictions Between Sources

Two authors' facts with the same subject and predicate, different objects and overlapping valid time are reported once; a later episode that overlaps neither is not.

## Query Provenance

Tests of per-row SPARQL provenance: which stored statements produced each solution. See [[query#Front Ends#SPARQL#Query Provenance]].

### Provenance Lists Matched Statements

A row lists the eids of its BGP statements, of a matched `OPTIONAL` part (none when absent), of the `UNION` branch taken, of annotation and reifier triples, and of each hop of a fixed-length path.

### Tested Statements Are Not Cited

Statements only tested by `FILTER EXISTS`, `FILTER NOT EXISTS` or `MINUS` are not listed, a virtual predicate adds no eid, and a recursive path region contributes nothing.

### Distinct Merges Provenance

`DISTINCT` merges rows equal on the projected variables and unions their eids; `LIMIT` and `OFFSET` after `DISTINCT` count merged rows in `ORDER BY` order, and `REDUCED` keeps each row.

### Groups And Subqueries Union Provenance

A group lists the union over its input rows (an empty list for a group over no rows), `HAVING` keeps it, and plain, `DISTINCT` and grouped subqueries carry their eids into the outer row.

### One Triple Lists All Its Eids

Several visible eids with one `(s, p, o)` give one row listing all of them, a reifier still gives one row per eid, and a valid-time view or a retraction narrows the list to the visible eids.

### Graphs Cite Memberships

`GRAPH <g>`, `GRAPH ?g` and a single `FROM <g>` list the statement and its `sys:inGraph` membership; several `FROM` graphs list the statement only.

### Time Scopes Cite Past Statements

A pattern in `SERVICE <tm:asOf/N>` lists the eid it matched then, even though that statement is retracted in the current view.

### Provenance Is Off By Default

`sparql` and `sparql_with` without provenance give equal results and byte-identical JSON; with it only the `"provenance"` member is added, and `ASK`, `CONSTRUCT` and updates fail with `Unsupported` and write nothing.

### Stale Answers Are Detectable

After a cited statement is retracted, it is missing from the current view's statements and `tm:txRetracted` on the history view names the retracting transaction.

### Provenance Keeps The Solutions

Every `SELECT` of the differential corpus returns the same rows with and without provenance, and every cited eid is visible in the view the query read.

### Provenance Is Sound

Property test: for random data with parallel statements and random BGPs, provenance never changes the rows, cited eids are visible, and a fresh store holding only the cited statements reproduces each row.

### Provenance Eids Keep Set Semantics

A `~prov` eid variable binds the canonical eid and keeps the canonical-eid predicate, so parallel statements still match once; a user eid variable matches each eid.

### Instrumentation Binds Hidden Eids

The provenance pass gives each stored pattern a hidden eid column, reuses a reifier's variable, and skips `EXISTS`, `MINUS` and virtual predicates.

### Provenance JSON Member

The SPARQL JSON document gains a `"provenance"` member between `head` and `results` only when provenance is present, and still parses as SPARQL 1.1 JSON results.

### Bridge Returns Provenance

The JSON bridge `sparql` operation with `"provenance": true` returns a `"provenance"` array parallel to `"rows"`, omits it otherwise, and rejects a non-boolean argument.

## Typed Layers

Checks of the `sys:subjectType` flag of [[data-model#Predicate Schema]] in the core, through both front ends and through the JSON bridge.

### Subject Type Constrains Subjects

A `sys:STMT` predicate accepts a statement subject and rejects a node with `SubjectTypeMismatch` naming the predicate, the tag IRIs and the tag, on assert and create, leaving no trace.

Several values mean any-of and accumulate, the flag takes effect within its transaction, and the check runs after the value type and before cardinality-one replacement.

### Subject Type Values Are Validated

The five subject tags are accepted as values, while a literal tag, an unknown tag, a datatype IRI or a string fails with `ValueTypeMismatch` for `sys:subjectType`, and a `sys:` subject is `ReservedNamespace`.

### Subject Type Changes Are Checked Against Live Data

The first value over node-level data, and retracting or superseding one of several values, fail with `SchemaConflict` listing the violating eids.

This holds for `retract_matching` too. Retracting the last value lifts the constraint, also after a schema read earlier in the same transaction.

### SPARQL Respects Typed Layers

`INSERT DATA` of a node-level triple on a `sys:STMT` predicate fails with `SubjectTypeMismatch` and the whole request writes nothing, while an annotation `{| v:confidence 0.8 |}` is accepted.

### Cypher Respects Typed Layers

`CREATE` and `SET` of a node property on a `sys:STMT` predicate fail with `SubjectTypeMismatch` and write nothing, while `SET r.confidence` on a relationship is accepted.

### Bridge Reports Subject Type Mismatch

A `transact` call that violates `sys:subjectType` fails with code `SubjectTypeMismatch` and commits none of its operations.

