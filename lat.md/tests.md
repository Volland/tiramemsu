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

## Dependents

Behaviour of [[time-model#Cascade#Dependents]], the read-only cascade walk on any view.

### Dependents Follow Layers

`dependents(e1)` lists e1, then its layers and references breadth-first in cascade order; plain nodes are not walked, a retracted layer is not walked, and an invisible root gives nothing.

### Dependents Terminate On Cycles

Two statements that reference each other, plus a statement linking them, are each listed once from either end, on the now and the history view.

### Dependents Follow The View

After a structure is retracted, `asOf` before the retraction lists it again, by transaction and by instant; `history` lists every statement that ever stood on the root, and `validAt` drops a layer valid in another period.

### Dependents Are Unbounded

A statement with 20 layers lists all 21 statements while a retraction with `max_cascade = 10` fails, and the read advances no id counter.

### Dependents Match The Dry Run

On random layered graphs (both hosts), `dependents(e)` on the now view equals the retracted set plus retracted memberships of a dry-run `retract(e)`, in the same order, and is empty for a retracted `e`.

The graphs mix layers, references, links between statements, reference cycles, memberships, confirmations, supersedes and retractions.

### Dependents Match Cascade And Path

Through the facade, for random layered graphs, `View::dependents(e)`, the dry-run retraction of `e` and the ends of `View::path(e, "(^sys:subject|^sys:object)*", REACH)` are the same set for every live statement.

### Dependents Reproduce The Past

Through the facade, the as-of and history views reproduce the dependents of a retracted structure (layer on a layer, a belief, a membership), and a speculation sees its own new layers.

## Fact Bundles

Behaviour of [[data-model#Fact Bundles]]: building a bundle from a view, importing it into another database, and its text forms.

### Bundle Collects Layers And Evidence

A bundle holds the root's layers, nested layers, beliefs and memberships with valid times, references first and ties by eid; a belief's bundle carries its evidence but not the evidence's other layers; output is stable; an invisible root is `NotLive`.

### Bundle Leaves Out Local Statements

Confirmations and layers on them, supersede links, and statements that reference a statement outside the view are left out; a root about a transaction or outside the view is `Unsupported`.

### Bundle Round Trip Between Databases

Importing into an empty database creates every statement, and the target's bundle of the imported root equals the source bundle: contents, layer structure, membership and valid times.

### Import Is Idempotent

A second import of the same bundle maps every id to the same eid with `new` false, asserts nothing, and leaves every table but `tx` unchanged.

### Import Attaches To An Existing Root

When the target already holds the root fact, the root maps to that eid and the layers are asserted on it; its valid time is not changed.

### Anonymous Nodes Stay Distinct

`NODE` and `BNODE` ids export as bundle-local labels; import mints fresh nodes that do not alias the target's, keeps a shared node shared and two nodes distinct, and mints again on a second import.

### Import Rejects Cycles And Malformed Bundles

A bundle on a reference cycle (or a self-reference) is refused with `Unsupported`, and skolem or local-id values, dangling references, duplicate ids, an unknown root and a literal predicate with `InvalidTerm`, leaving the target unchanged.

### Import Fails Atomically On Schema

Importing a statement that violates `sys:unique` in the target fails with `UniqueViolation` and stores nothing from the bundle.

### Bundle Of The Past

The as-of bundle of a structure retracted since imports as live copies whose bundle equals it.

### Bundles Round Trip On Random Graphs

On random layered graphs (both hosts), for every live statement whose bundle is not refused, import into an empty database then re-export gives the same bundle, and a second import is a no-op.

### Bundle JSON Round Trip

The `tiramemsu-bundle/1` JSON of a bundle with every term kind reads back equal and writes the same text; another version, missing fields, malformed terms, dangling references and skolem IRIs are `InvalidTerm`.

### Bundle N-Triples Is RDF 1.2

The N-Triples export parses as RDF 1.2 and holds each triple, its `rdf:reifies` reifier, layers as annotations on the reifier, valid-time bounds, and blank nodes for anonymous nodes.

### Bundle Travels As JSON Text

Through the facade, a belief's bundle written as JSON text and read back imports into a second database, where SPARQL finds the belief in its graph with its evidence.

### Bundles Cross The JSON Bridge

The bridge read `bundle` feeds a `transact` op `importBundle` on another database; `as` names the imported root, memberships and valid times arrive, and a malformed bundle fails.

## Recovery

Callback panics and failed read commits must release transactions and preserve reusable database connections without changing committed history.

### Transaction Panic

A caught transaction panic preserves its payload, rolls back facts and dictionary entries, and leaves the writer usable with gap-free transaction numbering.

### Speculative Panic

Panics in dry runs, speculative operations, and speculative queries leave no graph or event trace, preserve their payload, and burn allocated node, blank-node, term, and statement ids.

### Read Panic

Caught query panics on a pooled reader or the writer roll back the snapshot, return reader capacity, and permit repeated subsequent reads.

### Read Commit Failure

An injected read-commit error on either the writer or a pooled reader triggers rollback before connection reuse; the next read succeeds.

## Query Budgets

Opt-in budgets of [[query#Query Budgets]]: bounded reader waits, deadlines and cancellation of SQL and native work, atomic writes, and result budgets per operation.

### Pool Exhaustion Times Out

With the only reader held, a budgeted read fails with `PoolTimeout` after its reader timeout, `DeadlineExceeded` under a shorter deadline, or `Cancelled`; once released, reads succeed again.

### Default Reader Timeout

`OpenOptions::reader_timeout` (default `None`) bounds every read's wait with `PoolTimeout`, a budget overrides it, and without a pool the wait for the writer is bounded the same way.

### Reader Freed Before The Deadline

A read waiting under a long budget gets the reader once it is released and returns one committed state that includes a transaction committed while it waited.

### Cancel A Path

Cancelling or timing out a trail search on a dense graph (`View::path`, and Cypher `-[:knows*]->` through `tm_path`) stops it quickly with the typed error; the single reader is reusable at once.

### Native Checks Without Host Interrupts

On a host whose executors ignore `set_interrupt`, cancelling a native trail search still stops it during frontier expansion, through the path engine's own polling.

### Cancel SQL Execution

A cross join in SPARQL or Cypher stops with `DeadlineExceeded` or `Cancelled` through the progress handler; afterwards the reader carries no stale interrupt and runs a long plain query.

### Cancel A Write

A SPARQL update or Cypher write stopped by its deadline or token, and a budgeted transaction whose body outlives its deadline, commit no `tx` row, statement, term or event; later writes keep gap-free numbers.

### Result Overflow Fails

Past `max_rows` or `max_bytes`, SPARQL, Cypher, `triples`, `path` and `events_since` fail with `ResultLimitExceeded` instead of a truncated result, and an overflowing Cypher write commits nothing.

### Composite Operations Share One Budget

A row budget that fits a plain `SELECT` fails the same query with provenance, because its sibling lookups draw on the same meter; the statements of a multi-clause Cypher query do too.

### Default Budget Changes Nothing

`QueryBudget::default()` returns the same results as no budget, keeps the view's time selection and numbering, and budgeted reads still run in parallel on the pool.

### Bridge Budgets And Error Codes

The bridge's `budget` argument bounds reads, `cypherWrite` and `transact` with codes `DeadlineExceeded`, `ResultLimitExceeded` and `Cancelled`, rejects malformed budgets, and accepts `readerTimeoutMs`.

### Bridge Cancels A Running Call

`cancel` from another thread finds the running call by its `cancelKey`, stops it with `Cancelled`, and leaves other keys unaffected.

## Bulk Import

Opt-in import sessions of [[query#Bulk Import]]: chunks commit atomically under the write lease, statistics are refreshed once, and interruption keeps committed history.

### Later Chunk Fails

Two chunks commit and a third violates `sys:unique`: the first two stay visible with their transaction numbers, the third leaves no `tx` row, statement or term, and the next chunk keeps gap-free numbering.

### Retry An Assertion Chunk

Retrying an assertion chunk reuses the live statements by normal assertion semantics: the retry asserts nothing new and reports the same eids as `existing`.

### Finalize Many Chunks

With `optimize_every: 1`, twenty chunks run no analysis and write no STAT4 samples; `finish` runs one full `ANALYZE`, bumps the schema cookie for pooled readers, and restores the per-commit trigger.

### Analysis Failure

When another connection holds the write lock, the final `ANALYZE` fails with `Sqlite`; the summary lists both committed chunks and the maintenance error, the data stays, and statistics remain due until `optimize`.

### Cancel Between Chunks

Cancelling or dropping a session after a committed chunk keeps it (also across a reopen), runs no analysis, leaves statistics due, frees ordinary writes, and the next commit runs the upkeep.

### Read While Importing

A pooled reader on the importing thread or another thread, queried while a chunk is uncommitted, sees only the last committed chunk; SPARQL between chunks sees each committed one.

### Exclusive Write Lease

While a session lives, `transact`, SPARQL updates, Cypher writes and a second session fail with `ImportInProgress` from any thread; a shared session moves between threads, and a session cannot start inside a transaction.

### Budgeted Chunk

A chunk under a cancelled budget fails with `Cancelled`, leaves no trace and counts as rejected; a dry-run chunk commits nothing and is counted neither way.

### Bridge Import Sessions

The bridge's `importBegin`, `importChunk`, `importProgress`, `importFinish` and `importCancel` run a session by id, report progress and summaries, and map the lease error to `ImportInProgress`.

## Text Retrieval

Text recall of [[query#Text Recall]] over the derived index of [[storage#Text Index]]: view-aware visibility, inline strings, deterministic ranking with absent evidence, the index lifecycle and the entrypoints. Tests are in `text_retrieval.rs` and the bridge suite.

### Inline Text Is Recalled Without A Dictionary Row

A short string stored inline in its ObjectId is recalled like a dictionary string, keyed in `term_fts` by its full ObjectId, and neither recall nor a rebuild inserts a `term` row for it.

### Language Tagged Text Keeps Its Tag

A language-tagged string is recalled with its `lang`, diacritics fold (`cafe` matches `Café`), and typed literals are not searchable.

The all, any, phrase and prefix modes match as documented, query text is never FTS5 syntax, and wordless text is `InvalidQuery`.

### Retracted Text Leaves Now Recall

A matching statement retracted later is absent from the now view and from an as-of view at the retraction, and present in as-of views (by tx and by instant) before it and in history.

### Recall Honors Graph And Valid Time

With a graph list and a valid instant every hit is a visible member of a listed graph and valid at the instant; each filter alone, an empty graph list and a predicate filter behave as specified.

### Absent Confidence Is Reported As Absent

A hit without a confidence layer reports `None` and ranks after one with a layer at equal lexical score; a custom confidence predicate the hit lacks stays absent.

The largest numeric layer, confirmations, authors, `t_add` and its instant are reported.

### Equal Hits Keep Their Order

Hits with equal lexical score and evidence come back in ascending eid order on every repetition, and `limit` keeps the first hits by rank.

### Host Without FTS5 Rejects Recall

On a host that declares no FTS5, recall, rebuild and enable fail with `MissingCapability("fts5")`, no `term_fts` is created, and triple lookups, SPARQL and Cypher keep working.

### Writes Without FTS5 Are Caught Up

String statements written by a host without FTS5 set `meta.text_stale`; reopening on a host with FTS5 indexes them, clears the mark, and recall finds them.

### Stale Index Is Refused

Without a built index recall is `TextIndexUnavailable` and writes keep no index; enabling builds it once; a stale mark makes recall refuse until the next write with FTS5 closes the gap.

### Rebuild Restores Recall Without Touching History

After the index is emptied, a rebuild restores identical now and history recall, while every `triple`, `term` and `tx` row and the event log stay unchanged.

### Migration To Format 2 Keeps Every Row

A file written by a format-1 build migrates to format 2 on open with every `triple`, `term` and `tx` row unchanged, recall then covers its strings, and a format-1 build refuses the migrated file.

### Recall Sees Speculative Strings

Recall inside `Db::with` finds the speculative strings, and neither a speculation nor a dry run leaves an index row behind.

### Recall Honors Budgets

Recall is one budgeted operation: a row budget smaller than the hits fails with `ResultLimitExceeded` (also through SPARQL), a limit within it succeeds, and a cancelled token fails with `Cancelled`.

### SPARQL And Cypher Share The Recall

`tm:textMatch` with score, rank and confidence bindings and `CALL tiramemsu.text.search` return the same hits in the same order as `View::text_search`; limit, mode, graphs and time clauses apply, and misuse of the SPARQL patterns is `InvalidQuery`.

### Bridge Text Recall

The bridge's `textSearch`, `rebuildTextIndex`, `enableTextIndex` and `textIndex` open option return hits with evidence (`null` for absent confidence) and reach SPARQL and Cypher.

Errors map to `TextIndexUnavailable`, `ResultLimitExceeded`, `InvalidQuery` and `InvalidArgument`.

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

### Id Counters Are Bounded

With `next_stmt` seeded to 2⁴⁸ − 1 the statement is still allocated; at 2⁴⁸ assert and supersede fail with `IdSpaceExhausted { kind: STMT }` and leave no trace. `NODE`, `BNODE` and `TX` stop at 2⁴⁸ − 1 the same way.

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

### Origin And Counter Split

`origin()` and `counter()` split the payload of `NODE`, `BNODE`, `STMT` and `TX` at bit 48, at the bounds 2⁴⁸ − 1 and 2⁶⁰ − 1; local ids keep their old value and other tags have no origin. See [[data-model#ObjectId#Origin Bits]].

### Foreign Origin Is Rejected

An id with origin 1 is refused with `Unsupported` naming the origin: as a skolem IRI, as a `Value`, as a raw id or eid in every write operation, in a read lookup and in a bundle import. Failed writes leave no trace.

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

### Time Respecting Paths Match Brute Force

For random small graphs with random valid intervals and a random start instant, every mode equals an enumeration of all time-respecting walks within the hop bound.

`REACH` gives each end once with its shortest length and earliest arrival, `TRAIL` the edge-distinct walks with their arrivals, `ALL_SHORTEST` the minimal-length walks, and `ANY_SHORTEST` the lexicographically smallest of them per end. A property test.

### Time Respecting Scenarios

Hand cases of the hop rule, with the arrival each one expects in the modes it names.

An infection chain, a start instant that cuts early facts, same-instant chaining and its exclusive boundary, unbounded intervals, a longer walk that arrives earlier, a pair re-expanded with an earlier time, and shortest paths that avoid going back in time.

### Time Respecting Combines With Graphs And Layers

Virtual hops through layers keep the time, a stored hop after them still needs its fact to hold, a graph set and the view's `validAt` still filter every hop, and the arrival is the same through every surface.

### tm_path Arrival Column

`tm_path` makes a call time-respecting through a `timeRespecting` view part, returns `arrival` for `REACH` and `TRAIL` rows (NULL without the option), matches `View::path_with`, and rejects a malformed or repeated part with `tm_path: view:`.

### Time Respecting View Text

The `view` text parser accepts `timeRespecting` alone, with an RFC 3339 date or date-time or epoch milliseconds, in any order with the other parts and with the `tm:` prefix, and rejects a repeated or malformed part.

## Named Graphs

Checks of [[data-model#Named Graphs]] across the core, the planner, SPARQL query and update, and Cypher.

### Membership Is Engine Owned

Asserting `sys:inGraph` through `Tx::assert` or `create` fails with `ReservedNamespace`, and a schema statement cannot be added to a graph. Nothing is written in either case. See [[data-model#Named Graphs]].

### Graph Names Are Validated

A literal or a transaction as graph name fails with `InvalidGraphName` on every graph method and writes nothing, while IRI, `NODE` and `BNODE` names work.

### Error Messages Name The Term

`InvalidGraphName`, `GraphNotFound` and `GraphExists` render messages that name the rejected term or graph.

### Statement Can Be In Many Graphs

One statement added to two graphs keeps one eid and gets two memberships, adding twice is idempotent, and a retracted statement cannot be added. The report lists memberships apart from statements.

### Remove And Clear Keep Statements

`remove_from_graph` and `clear_graph` retract memberships only, removing a non-member is a no-op, `create_graph` is idempotent, and `drop_graph` retracts the declaration and keeps other metadata.

### Views Apply To Membership

`graph_members` and `graphs` honour `asOf`, `history` and `validAt` for the membership, and the member statement must be visible in the same view.

### Cascade And Supersede Drop Memberships

Retracting a statement retracts its memberships with `ret_kind` cascade in the same transaction. Supersede and cardinality-one replacement do not copy a statement's own memberships to the replacement.

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

### Property Paths Run Inside Graphs

A recursive path under `GRAPH <g>`, `GRAPH ?g` (enumerated, bound by a triple pattern, or bound outside the block), `FROM` and `FROM NAMED` follows only member statements, also in an update `WHERE`.

A non-recursive path puts the graph on each triple of its translation, and the same path outside a graph reads the union.

### Zero Length Paths Per Graph

A nullable path gives its zero-length match once per graph in scope: once per graph under `GRAPH ?g` (also for a term in no statement), once under `GRAPH <g>` even for a graph with no member, and once for a `FROM` default graph.

### Graph Paths Under asOf

After a membership is removed, `GRAPH <g>` paths stop at it now, `SERVICE <tm:asOf/t>` still follows it, and `GRAPH ?g` enumerates the graphs of the view it runs in.

### Paths Cross Layers Inside Graphs

Virtual `sys:subject` and `sys:object` hops inside `GRAPH <g>`, forward and inverse, step only from and to statements that are members of `g`, and `GRAPH ?g` binds the one graph the whole path lies in.

### Graph Scoped Paths Follow Membership

Through `View::path_with`, a graph set confines every mode to member statements. A statement in two listed graphs is one hop, zero-hop rows ignore the set, and memberships are read in the hop's view (`asOf`, `history`).

### Graph Scoped Virtual Hops

A virtual hop needs the statement whose part it steps to or from in the graph set, in both directions, and a store that never wrote `sys:inGraph` gives only zero-hop rows.

### Graph Filter Fetch Uses An Index Seek

With a graph set, every fetch shape under `now`, `asOf` and `history` holds the membership `EXISTS` with no data in its text, and `EXPLAIN QUERY PLAN` shows a covering-index seek for the membership. The SQL and plans match `insta` snapshots.

### tm_path Graphs Argument

`tm_path` takes one graph as an integer, several as JSON text, NULL as no filter, `'[]'` as the empty set and a column as a correlated graph, keeps the pushed-down end, and rejects a malformed `graphs` with `tm_path: graphs:`.

### Path Graph Selector Text And Validation

The IR text form of a path prints `:graph` for a set or a variable and nothing for `Any`, the variable is in scope, and an empty set or a variable in a set is invalid.

### Path Graph Selector SQL Snapshots

The planner passes a set of one graph as an integer and several as JSON text, correlates a variable bound by a joined pattern and enumerates an unbound one, matching `insta` snapshots, and a path without a selector keeps its five-argument call.

### Graph Names Are Rejected When Invalid

A transaction IRI as graph name, in `GRAPH`, `FROM`, `CREATE` or a template, and a graph variable bound to a literal, fail with `InvalidGraphName` and write nothing.

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

### A Statement Holds A Subgraph

A live statement is a graph name: `add_to_graph` with a statement graph is idempotent, `remove_from_graph` and `clear_graph` keep the members, and a retracted statement graph fails with `NotLive` and writes nothing.

### Retracting A Statement Graph Keeps Its Members

Retracting a statement that names a graph cascades to its memberships in the same transaction and leaves the members live; `asOf` before the retraction still shows the members.

### Supersede Carries The Contents Of A Statement Graph

Superseding an edge replays the memberships of its graph onto the new eid with the members keeping their eids, while the edge's own membership in another graph and a superseded member's membership are dropped.

### SPARQL Statement Graphs

`GRAPH <urn:tiramemsu:stmt:n>`, `GRAPH ?e` bound by `~ ?e`, `FROM`, a template graph bound to a statement and `CLEAR GRAPH` work on a statement graph; deleting the edge empties the graph, keeps the members, and blocks new contents.

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

### Metagraph Containers Nesting And Fold

Nested graphs through `within*` and `GRAPH ?g` (also in Cypher, under a cycle and `asOf`), fold by `INSERT … WHERE`, an edge holding a subgraph through a supersede and a retraction, and n-ary role statements.

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
