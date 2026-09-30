# Tests

Test specifications for the invariants the design depends on. Each leaf is one test or one property test, and implementation code references it with an `@lat:` comment.

Every leaf except the Query ones is referenced from the `tm-core` tests. The Query leaves belong to M1–M3, so the `require-code-mention: true` frontmatter is added once the last of those changes lands.

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

The manifest-driven runner executes the in-scope SPARQL 1.0, 1.1 and 1.2 test categories against fresh databases. It fails on any failure that is not in `expected-deviations.toml`, and on any listed test that now passes.

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
