# Overview

Tiramemsu is an embedded graph database on SQLite: addressable triples, Datomic-style time travel plus valid time, and SPARQL and Cypher over one store.

The full option analysis and interview record live in `docs/design-options.md`. This knowledge graph holds the **decided** design only.

## Goals

The database targets agent and personal memory: facts with provenance, beliefs about facts, and exact answers to "what did we know, and when".

- Every triple has an identity (eid), so it can carry properties and be referenced by other triples, to any depth. See [[data-model#Statements]].
- Every change is kept forever with its transaction time and operation. See [[time-model]].
- A fact carries the valid-time interval of when it was true in the world. See [[time-model#Valid Time]].
- SPARQL and Cypher both query the same store with the same results. See [[query#Front Ends]].
- It runs in-process on SQLite, with a Rust core that reaches SQLite through a small executor trait. See [[architecture]] and [[architecture#Executor]].

## Capabilities in 0.3

Release 0.3.0 turns the store into an agent memory: bounded, searchable, reviewable, and reachable from Rust, Node, Python, a browser and any MCP client. The map with entry points is [[architecture#Agent Memory Features]].

- **Bounded calls:** deadlines, cancellation, reader timeouts and row or byte limits per operation, with typed errors and no partial results ([[query#Query Budgets]]).
- **Large loads:** bulk import sessions commit chunks under a write lease and analyse statistics once ([[query#Bulk Import]]).
- **Recall by words:** FTS5 over stored strings, ranked by lexical score and then by the evidence layers the graph already holds ([[query#Text Recall]]).
- **Trust over time:** saved answers record what they cited and turn later events into `recheck` and `stale` marks ([[query#Saved Answers]]).
- **Disagreement made visible:** conflict inspection and dry-run bundle previews, never an automatic winner ([[query#Conflict Inspection]]).
- **Journeys in queries:** time-respecting paths in SPARQL and Cypher, and every path search says whether it was complete ([[query#Temporal Path Syntax]]).
- **Cyclic joins:** an opt-in leapfrog triejoin for skewed cyclic patterns, with explained routing ([[query#Physical Planning#LFTJ]]).
- **Small embeds and new hosts:** the query engine and front ends are cargo features, and the engine runs on SQLite compiled to WebAssembly ([[architecture#Crates#Cargo Features]], [[architecture#WebAssembly Host]]).
- **Agent tools:** the `tiramemsu-mcp` stdio server exposes typed, auditable memory tools ([[api#MCP Tools]]).

The file format is 3 ([[storage#Format Versioning]]); older files migrate on open without rewriting history.

## Non-Goals

These are explicitly out of scope, so the design can stay small and exact.

- Server-grade analytics at 10⁹+ triples, where custom storage (MillenniumDB, Kùzu) wins.
- Physical deletion or excision of facts. See [[time-model#Never Forget]]. Legal erasure is served by crypto-shredding instead, which deletes no row. See [[time-model#Erasure]].
- Persistent git-style branches. Only short speculative transactions exist. See [[time-model#Speculative Transactions]].
- Multi-writer replication in v1. The event log is kept ready for it. See [[time-model#Event Log]].
- Datalog as a query language in v1.

## Decision Record

Every row is a decision taken in the design interview of 2026-09-29, with the section that specifies it. Rows D29–D34 were added on 2026-09-30 with the layer features collected in [[recipes]].

Rows D19–D25 were added the same day, after a comparison with oxilite ([[prior-art#oxilite]]) and after measuring SQLite 3.53 plans on the schema ([[query#Physical Planning#Join Ordering]], [[storage#Measured Footprint]]). D26 follows a SQLite versus DuckDB benchmark on the same workload ([[prior-art#DuckDB]]).

D35 was added on 2026-10-01: it reserves origin bits so that agent files can later merge without rewriting ids. D36 was added the same day: a statement can name a graph, so an edge can hold a subgraph.

| ID | Decision | Specified in |
|---|---|---|
| D1 | Scope: embedded agent / personal memory (10⁴–10⁷ triples) | [[overview#Goals]] |
| D2 | Rust core with bundled SQLite via `rusqlite`, the first executor host (D22); M0 built directly in Rust | [[architecture#Crates]] |
| D3 | "Layered" means layers built on triple ids (metagraph); no named-graph key column | [[data-model#Layers]] |
| D4 | Uniform statements: properties are triples with eids too | [[data-model#Statements]] |
| D5 | `assert` is idempotent; `create` always mints a new eid | [[time-model#Operations#Assert]] |
| D6 | Eids may appear as s or o of other triples; no direct self-reference | [[data-model#Layers]] |
| D7 | Valid time as immutable `v_from`/`v_to` columns on the triple | [[time-model#Valid Time]] |
| Q10 | `supersede` corrects a fact by cascade-and-replay | [[time-model#Operations#Supersede]] |
| D8 | Retract cascades recursively over subject and object positions | [[time-model#Cascade]] |
| D9 | One `triple` table carries its own lifetime `t_add`/`t_ret`; ids never reused | [[storage#Triple Table]] |
| D10 | 64-bit ObjectId with a 4-bit low tag | [[data-model#ObjectId]] |
| D12 | Hybrid execution: SQL codegen + native paths + opt-in LFTJ | [[query#Physical Planning]] |
| D13 | SPARQL and Cypher built in parallel over one IR with differential tests | [[query#Front Ends]] |
| Q14 | Cypher dual view: an eid is both a relationship and a `:Statement` node | [[query#Front Ends#Cypher Dual View]] |
| Q15 | One `@vocab` base plus a versioned prefix table | [[data-model#Vocabulary Mapping]] |
| D14 | Time in the API and in queries, scoped per pattern | [[query#Temporal Syntax]] |
| D15 | Paths in queries, the API and a SQL table function; virtual `sys:subject`/`sys:object` hops | [[query#Physical Planning#Path Engine]] |
| D16 | Optional predicate schema stored as `sys:` triples | [[data-model#Predicate Schema]] |
| D17 | Never forget: no DELETE on triples or terms | [[time-model#Never Forget]] |
| Q20 | High-churn state goes in a `volatile` side table, not the graph | [[storage#Volatile Table]] |
| D18 | Speculative `with` via SQLite SAVEPOINT; no branches | [[time-model#Speculative Transactions]] |
| D19 | Planner statistics are mandatory and kept current automatically; an engine-forced join order is a benchmark-gated fallback | [[query#Physical Planning#Join Ordering]] |
| D20 | `DATETIME` keeps its timezone offset inside the inline payload; numbers still collapse to their value | [[data-model#ObjectId#Canonical Encoding]] |
| D21 | SPARQL scopes time per group with `SERVICE <tm:…>`, not `GRAPH`, so `GRAPH` stays free for named graphs | [[query#Temporal Syntax]] |
| D22 | The core reaches SQLite through a synchronous executor trait with declared capabilities | [[architecture#Executor]] |
| D23 | Reuse from oxilite: its Cypher parser as the fallback, its allow-list test harnesses, and its write-cost and as-of benchmarks | [[prior-art#oxilite]] |
| D24 | Crypto-shredding is scheduled (M6); format 1 reserves tag 15 and `sys:sensitive` for it | [[time-model#Erasure]] |
| D25 | Retrieval (FTS5 over the term dictionary, vectors on hosts that have them) is a planned milestone; the FTS5 part shipped in 0.3.0 | [[roadmap#Milestones]], [[query#Text Recall]] |
| D27 | SPARQL removes duplicate `(s, p, o)` only for predicates recorded in `pred_multi`; the eid stays as the statement identity | [[storage#Multi-Eid Predicates]] |
| D28 | Named graphs are tags: a graph is a node and membership is a layer statement `(e sys:inGraph g)`, so there is no quad column; the default graph is the union | [[data-model#Named Graphs]] |
| D26 | SQLite stays the engine. DuckDB was benchmarked on the workload and is only an optional read-only analytics tool over the SQLite file | [[prior-art#DuckDB]] |
| D29 | `tm:addedAt` / `tm:retractedAt` expose the commit instants of a statement as virtual predicates; Cypher reads them as `r.addedAt` / `r.retractedAt` | [[query#Views and Scans#Virtual Predicates]] |
| D30 | `sys:subjectType` constrains the subject kind of a predicate (typed layers); several values mean any of them | [[data-model#Predicate Schema]] |
| D31 | SPARQL provenance is opt-in per query and never changes the rows; it cites what matched, not what a filter tested | [[query#Front Ends#SPARQL#Query Provenance]] |
| D32 | `dependents` is the cascade walk as an untruncated read on any view; fact bundles build on it and import by assert | [[time-model#Cascade#Dependents]], [[data-model#Fact Bundles]] |
| D33 | Recursive paths filter by graph membership on every traversed statement, in the path's own view | [[query#Physical Planning#Path Engine]] |
| D34 | Time-respecting paths use earliest-arrival semantics: a hop needs its fact to hold at the walk's time, which only moves forward | [[query#Physical Planning#Path Engine#Time-Respecting Search]] |
| D35 | The high 12 payload bits of `NODE`, `BNODE`, `STMT` and `TX` are a reserved origin (format 1 writes 0 and rejects others); counters stop at 2⁴⁸ − 1 | [[data-model#ObjectId#Origin Bits]] |
| D36 | A statement eid is a graph name: an edge holds a subgraph, its memberships cascade with it, and supersede replays them onto the new eid | [[data-model#Named Graphs#Statement Graphs]] |

## Open Inputs

One fact is still unknown, and one has been settled by building every option. They affect priorities, not the design.

- **Scale ceiling:** triples per database and writes per second. This sets benchmark targets and how far LFTJ routing is tuned; LFTJ itself is built and opt-in, justified by the skewed triangle benchmark. See [[roadmap#Benchmarks]].
- **First consumer:** no longer a choice. The MCP server, Python, Node.js and WASM bindings all ship over one JSON bridge ([[api#Bindings]], [[bindings]]).

## Formal Model

The paper `paper/layered-bitemporal-graphs.tex` formalises this design and proves what its operations guarantee. It also shows how metagraphs inherit both clocks.

Its central operator is the ground core γ(X), the largest subset of X whose statements' referents all lie in X. Each result is stated against a section of this knowledge graph:

- The cascade retracts exactly X ∖ γ(X∖{e}), the least retraction that keeps memory grounded ([[time-model#Cascade]]).
- Every `asOf` view is grounded iff transaction lifetimes nest along references ([[time-model#Transaction Time]]). Valid time does not nest by design. Its largest grounded slice is the intersection of intervals over a statement's support ([[time-model#Valid Time]]).
- Supersede is an isomorphism on the ground core of its cascade set ([[time-model#Operations#Supersede]]).
- Metagraphs embed through membership statements that may be named by statements ([[data-model#Named Graphs#Statement Graphs]]). Snapshots commute with the encoding for well-formed temporal metagraphs ([[recipes#Metagraphs]]).

The paper review found four conformance gaps: `assert` accepted statements that were not live in subject or object position, and eids that were never allocated; supersede replayed layers of dropped memberships onto eids never inserted; and raw writes could backdate statement dates. All four are repaired in 0.4.0 (live endpoints, closed retention on correction, and the transaction-date guards of storage format 4), with regression tests in [[tests#Conformance Repairs]]. Version 0.3.0 still has them; see `paper/README.md`.
