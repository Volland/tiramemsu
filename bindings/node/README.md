<p align="center"><img src="https://cdn.jsdelivr.net/gh/Volland/tiramemsu@main/site/assets/logo.svg" width="140" alt="Tiramemsu: a brain taking a bite out of a tiramisu whose layers are a graph"></p>

<h1 align="center">@tiramemsu/node</h1>

<p align="center"><em>Your agent's brain loves Tiramemsu.</em><br>
Layered, never-forget memory for agents, for Node.js.</p>

<p align="center"><a href="https://volland.github.io/tiramemsu/">Website</a> · <a href="https://volland.github.io/tiramemsu/articles/time-travel.html">Time travel explained</a> · <a href="https://volland.github.io/tiramemsu/articles/layered-graphs.html">Layered graphs explained</a> · <a href="https://github.com/Volland/tiramemsu">GitHub</a></p>

Tiramemsu is an embedded graph database on SQLite. Every fact has its own id, so a fact can carry a confidence, a source or a belief about it, layer on layer. Every change is kept, with when the database learned it and when it was true in the world. This package is the Node.js binding: a native addon and a typed, synchronous TypeScript API.

> **Status: new.** Tiramemsu was designed and implemented in September 2026. This package has not been published to npm yet, so the install line below works after the first release. Until then, build it from source.

## Install

```sh
npm install @tiramemsu/node
```

Node 18 or later. The package ships a prebuilt native addon for Linux (x64, arm64), macOS (x64, arm64) and Windows (x64). On any other platform the import fails with a message that names it.

<details>
<summary>Build from source instead</summary>

You need Node 18 or later and the Rust toolchain pinned by the repository (1.91.1).

```sh
git clone https://github.com/Volland/tiramemsu
cd tiramemsu/bindings/node
npm ci
npm run build:native   # cargo build --release, then copies the addon next to package.json
npm run build          # compiles the TypeScript to dist/
npm test
```

If another `rustc` is first on your `PATH` (for example Homebrew's), put rustup's first: `export PATH="$HOME/.cargo/bin:$PATH"`.

</details>

## Quick start

Alice joins Acme in 2020, with a confidence of 0.8. We later learn she left at the end of 2023, and then that she joined Globex in March 2024.

```ts
import { Database, iri } from "@tiramemsu/node";

const v = (name: string) => iri(`urn:tiramemsu:v:${name}`); // `v:` in queries
const [alice, worksAt, acme, globex, confidence] =
  ["alice", "worksAt", "acme", "globex", "confidence"].map(v);

const db = Database.open("memory.db");

// 1. A fact is a statement with its own id, so it can carry a layer.
const first = db.transact((tx) => {
  const job = tx.assert(alice, worksAt, acme, { validFrom: "2020-01-01" });
  tx.assert(job, confidence, 0.8); // `job` is a Ref: the statement, before it has an id
});

// 2. Correct it. The layer moves to the new statement; nothing is deleted.
db.transact((tx) => {
  tx.supersede(first.asserted[0], { validTo: "2024-01-01" });
});

// 3. A new fact.
db.transact((tx) => {
  tx.assert(alice, worksAt, globex, { validFrom: "2024-03-01" });
});

// 4. Ask the same question at different times.
const q = "SELECT ?org WHERE { v:alice v:worksAt ?org }";
const orgs = (view: ReturnType<typeof db.now>) => {
  const r = view.sparql(q);
  return r.kind === "select" ? r.rows.map((row) => (row.org as { iri: string }).iri.split(":").pop()) : [];
};

orgs(db.now());                                    // ["acme", "globex"]  both episodes are live
orgs(db.asOf({ tx: 1 }));                          // ["acme"]            what we believed after tx 1
orgs(db.asOf({ tx: 1 }).validAt("2026-01-01"));    // ["acme"]            we thought she was still there
orgs(db.now().validAt("2026-01-01"));              // ["globex"]          what is true today
orgs(db.now().validAt("2024-02-01"));              // []                  between the two jobs
```

The same store speaks Cypher, and a layer is a relationship property:

```ts
db.now().cypher("MATCH (a)-[r:worksAt]->(c) RETURN c, r.confidence AS conf");
// { columns: ["c", "conf"], rows: [[acme, 0.8], [globex, null]] }  (Globex has no confidence layer)
```

## Two clocks

A view chooses when you look and when the fact was true. Every read takes a view, and views are immutable.

| You want | Code |
|---|---|
| What we believe now | `db.now()` |
| What we believed after transaction `n` | `db.asOf({ tx: n })` |
| What we believed at a wall-clock time | `db.asOf({ instant: "2026-09-01T12:00:00Z" })` |
| What was true on a date | `view.validAt("2026-01-01")` |
| Every statement ever, with retracted ones | `db.history()` |

`validAt` combines with any of them. Valid time is off unless you ask for it. [Time travel and bitemporality, explained](https://volland.github.io/tiramemsu/articles/time-travel.html) has the full story.

## Writing

`db.transact(fn)` collects the operations of the callback and commits them as one transaction. If the callback throws, nothing is committed. `assert`, `create` and `supersede` return a `Ref` that later calls in the same callback accept as a statement id, a subject or an object.

| Method | What it does |
|---|---|
| `assert(s, p, o, opts?)` | Adds a statement; idempotent when a live one with an overlapping valid time exists |
| `create(s, p, o, opts?)` | Always adds one, for parallel edges |
| `retract(eid)` | Ends belief in a statement and every layer about it |
| `retractMatching({ s?, p?, o? })` | Retracts every live match |
| `supersede(eid, { o?, validFrom?, validTo? })` | Corrects a statement and replays its layers on the new one |
| `confirm(eid)` | Records that another source agrees |
| `meta(p, o)`, `upsert(p, o)`, `newNode()` | Transaction metadata, unique upsert, an anonymous node |
| `addToGraph(eid, g)`, `removeFromGraph`, `clearGraph`, `createGraph`, `dropGraph` | Named graphs as tags on statements. A statement (`stmt(eid)` or a `Ref`) can name a graph, so an edge can hold a subgraph |
| `importBundle(bundle)` | Imports a bundle from `view.bundle`, possibly read from another database; returns a `Ref` to the imported root |
| `cypher(text, params?)` | A Cypher query that may write, inside the same transaction |

`opts` takes `validFrom`, `validTo` (a `Date`, epoch milliseconds or an RFC 3339 string) and `onExisting: "confirm"`. `transact(fn, { dryRun: true })` reports what would happen and commits nothing. `db.speculate(fn, queries)` applies changes hypothetically, runs queries on the result, and keeps nothing.

## Reading

| Method | Returns |
|---|---|
| `view.sparql(text, { provenance? })` | `{ kind: "select", vars, rows, provenance? }`, `{ kind: "ask", value }`, `{ kind: "graph", triples }` or `{ kind: "update", report }` |
| `view.cypher(text, params?)` | `{ columns, rows }` |
| `view.triples({ s?, p?, o? })` | Statements with `eid`, the three terms, `tAdd`, `tRet`, `validFrom`, `validTo`, `retKind` |
| `view.path(start, expr, { mode?, maxHops?, graphs?, timeRespecting? })` | Endpoints with hop counts and `arrival`. `mode` is `reach`, `trail`, `anyShortest` or `allShortest` |
| `view.dependents(eid)` | The statements that stand on `eid`: what retracting it would cascade to |
| `view.bundle(eid)` | The statement with its layers and evidence, as `tiramemsu-bundle/1` JSON |
| `view.events(since?)` | The change log after a transaction |
| `view.graphs()`, `view.graphMembers(g)`, `view.values(s, key)` | Named graphs and values |

`expr` is SPARQL property-path syntax with the predeclared prefixes `v:`, `sys:`, `tm:`, `rdf:` and `xsd:`, for example `v:knows+`. SPARQL supports RDF 1.2 annotations: `{| v:confidence ?c |}` reads a layer.

With `provenance: true` a SELECT result also carries, for each row, the statements (`{ stmt }` terms) that produced it. `graphs` keeps every hop of a path inside the listed graphs. `timeRespecting: true` (or `{ after: time }`) makes each hop start no earlier than the previous one, and every row then carries its earliest `arrival` in epoch milliseconds.

## Terms

| JavaScript | Stored as |
|---|---|
| `string`, `boolean` | plain string, boolean |
| integer `number`, `bigint` | `xsd:integer` (any size; beyond 2^53 comes back as `bigint`) |
| fractional `number` | `xsd:double` |
| `Date` | `xsd:dateTime` (comes back as `Date`) |
| `iri(s)` | IRI |
| `literal(lex, { datatype })`, `literal(lex, { lang })` | typed or language-tagged literal |
| `node(n)`, `bnode(n)`, `stmt(eid)`, `tx(n)` | anonymous node, blank node, statement id, transaction |

## Errors

Every failure throws a `TiramemsuError` with a `code`:

| Code | Meaning |
|---|---|
| `Parse` | SPARQL, Cypher or path text is invalid |
| `InvalidArgument` | The call itself is malformed: an unknown operation, a missing field, a term of the wrong shape |
| `NotLive` | The statement does not exist or was already retracted |
| `Unsupported` | Valid but outside the supported subset, such as a Cypher write through a read view |
| `UniqueViolation` | A `sys:unique` predicate already has a live holder of that value |
| `ValueTypeMismatch` | The object does not match the predicate's `sys:valueType` |
| `SubjectTypeMismatch` | The subject does not match the predicate's `sys:subjectType`, such as a layer predicate on a plain node |
| `CascadeLimitExceeded` | A retraction would touch more than `maxCascade` statements |
| `PathLimitExceeded` | A path search exceeded its state limit |
| `DeadlineExceeded` | A budgeted call ran past its `timeoutMs` |
| `Cancelled` | A budgeted call was stopped with `db.cancel(key)` |
| `PoolTimeout` | No reader became free within `readerTimeoutMs` |
| `ResultLimitExceeded` | A budgeted call decoded more than `maxRows` rows or `maxBytes` bytes |
| `InvalidPatch` | A `supersede` patch tried to change the subject or predicate |
| `IdSpaceExhausted` | An id counter passed 2^48 − 1 |
| `ImportInProgress` | A write, or a second import session, while a bulk import session is open |
| `Sqlite` | A SQLite failure, such as a locked or unreadable file |

## Options

`Database.open(path, options?)` accepts `readers`, `busyTimeoutMs`, `termCacheCapacity`, `optimizeEvery`, `pathMaxHops`, `pathMaxStates` and `readerTimeoutMs`. Anything else is rejected.

## Budgets

Calls are unbounded by default. A budget bounds one call: `timeoutMs`, `readerTimeoutMs`, `maxRows` and `maxBytes` (counted across every statement the call runs), and `cancelKey`. A stopped write commits nothing, and an over-budget result throws instead of returning a prefix.

```ts
const view = db.now().withBudget({ timeoutMs: 500, maxRows: 10_000 });
view.sparql("SELECT ?o WHERE { <urn:ex:alice> <urn:ex:worksAt> ?o }");
db.transact((tx) => { tx.assert(alice, worksAt, acme); }, { budget: { timeoutMs: 1_000 } });
db.cypherWrite("CREATE (:Person {name: 'Bob'})", undefined, { budget: { timeoutMs: 1_000 } });
```

`db.cancel(key)` stops the call running with that `cancelKey`. The API is synchronous, so from the same thread it only stops a call that starts later; use a fresh key per call.

## Bulk import

`db.bulkImport()` starts a session for large loads: each `chunk` is one atomic transaction (a function of `tx` or a list of op objects, with the same options as `transact`), chunks skip the per-commit statistics refresh, and `finish()` runs one full analysis. A failing chunk throws and rolls back alone; earlier chunks stay. While the session is open other writes throw `ImportInProgress` and reads see the last committed chunk.

```ts
const imp = db.bulkImport();
try {
  for (const batch of batches) {
    imp.chunk((tx) => { for (const [s, p, o] of batch) tx.assert(s, p, o); });
  }
} finally {
  const summary = imp.finish(); // { progress, analyzed, statisticsDue, maintenanceError }
  console.log(summary.progress.chunks, summary.progress.asserted, summary.progress.txs);
}
```

`imp.progress()` reports `chunks`, `rejected`, `asserted`, `existing`, `retracted`, `txs`, `elapsedMs` and `maintenanceMs`. `imp.cancel()` ends the session without analysis and keeps every committed chunk; `db.info().statisticsDue` then stays true until the next write or `db.optimize()`. Always end a session, since it holds the write lease until `finish()` or `cancel()`.

## Good to know

- The API is synchronous, like the Rust core. Reads run in parallel on a small connection pool, and writes serialize on one writer.
- Nothing is ever deleted: the SQLite file rejects `DELETE`. Forgetting means retracting, and the past stays exact.
- Results cross the native boundary as JSON, which is fine for agent memory and not meant for million-row result sets.
- There is no MCP server, WASM build or network server yet.

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
