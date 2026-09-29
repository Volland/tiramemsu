# Tests

Test specifications for the invariants the design depends on. Each leaf is one test or one property test, and implementation code references it with an `@lat:` comment.

No code exists yet. When the first tests land, add the `require-code-mention: true` frontmatter to this file, so `lat check` enforces that every spec is covered.

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

## ObjectId

Checks of the codec in [[data-model#ObjectId]].

### Canonical Round Trip

For every tag, encode then decode returns the value. A value that can be inlined never gets a dictionary id.

### Order Within Tag

For INT, DATE and DATETIME, integer order of the ObjectIds equals value order, negative values included, under SQLite's signed comparison.

## Query

Checks of [[query#Front Ends]] and [[query#Physical Planning#Path Engine]].

### Differential SPARQL Cypher

A corpus of equivalent SPARQL/Cypher query pairs over shared fixtures returns identical result multisets, after normalising for set-versus-bag semantics.

### Dual View Binds Same Eid

A Cypher relationship variable used in node position and a SPARQL `~ ?r` reifier bind the same eid for the same fact.

### Per Pattern Time Scopes

A query that mixes an `asOf(t)` pattern and a now pattern returns the before and after values of a superseded fact.

### Paths Cross Layers

A path using `sys:subject` hops finds the entities behind a statement that a belief references.

### Unbounded Paths Are Capped

A Cypher `-[*]->` pattern without an upper bound stops at `max_hops` and does not fail.
