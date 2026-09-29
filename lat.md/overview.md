# Overview

Tiramemsu is an embedded graph database on SQLite: addressable triples, Datomic-style time travel plus valid time, and SPARQL and Cypher over one store.

The full option analysis and interview record live in `docs/design-options.md`. This knowledge graph holds the **decided** design only.

## Goals

The database targets agent and personal memory: facts with provenance, beliefs about facts, and exact answers to "what did we know, and when".

- Every triple has an identity (eid), so it can carry properties and be referenced by other triples, to any depth. See [[data-model#Statements]].
- Every change is kept forever with its transaction time and operation. See [[time-model]].
- A fact carries the valid-time interval of when it was true in the world. See [[time-model#Valid Time]].
- SPARQL and Cypher both query the same store with the same results. See [[query#Front Ends]].
- It runs in-process on SQLite, with a Rust core. See [[architecture]].

## Non-Goals

These are explicitly out of scope, so the design can stay small and exact.

- Server-grade analytics at 10⁹+ triples, where custom storage (MillenniumDB, Kùzu) wins.
- Physical deletion or excision of facts. See [[time-model#Never Forget]].
- Persistent git-style branches. Only short speculative transactions exist. See [[time-model#Speculative Transactions]].
- Multi-writer replication in v1. The event log is kept ready for it. See [[time-model#Event Log]].
- Datalog as a query language in v1.

## Decision Record

Every row is a decision taken in the design interview of 2026-09-29, with the section that specifies it.

| ID | Decision | Specified in |
|---|---|---|
| D1 | Scope: embedded agent / personal memory (10⁴–10⁷ triples) | [[overview#Goals]] |
| D2 | Rust core on `rusqlite` with bundled SQLite; M0 built directly in Rust | [[architecture#Crates]] |
| D3 | "Layered" means layers built on triple ids (metagraph); no named-graph key column | [[data-model#Layers]] |
| D4 | Uniform statements: properties are triples with eids too | [[data-model#Statements]] |
| D5 | `assert` is idempotent; `create` always mints a new eid | [[time-model#Operations#Assert]] |
| D6 | Eids may appear as s or o of other triples; no direct self-reference | [[data-model#Layers]] |
| D7 | Valid time as immutable `v_from`/`v_to` columns on the triple | [[time-model#Valid Time]] |
| Q10 | `supersede` corrects a fact by cascade-and-replay | [[time-model#Operations#Supersede]] |
| D8 | Retract cascades recursively over subject and object positions | [[time-model#Cascade]] |
| D9 | One `triple` table carries its own lifetime `t_add`/`t_ret`; ids never reused | [[storage#Triple Table]] |
| D10 | 64-bit ObjectId with a 4-bit low tag | [[data-model#ObjectId]] |
| D12 | Hybrid execution: SQL codegen + native paths + deferred LFTJ | [[query#Physical Planning]] |
| D13 | SPARQL and Cypher built in parallel over one IR with differential tests | [[query#Front Ends]] |
| Q14 | Cypher dual view: an eid is both a relationship and a `:Statement` node | [[query#Front Ends#Cypher Dual View]] |
| Q15 | One `@vocab` base plus a versioned prefix table | [[data-model#Vocabulary Mapping]] |
| D14 | Time in the API and in queries, scoped per pattern | [[query#Temporal Syntax]] |
| D15 | Paths in queries, the API and a SQL table function; virtual `sys:subject`/`sys:object` hops | [[query#Physical Planning#Path Engine]] |
| D16 | Optional predicate schema stored as `sys:` triples | [[data-model#Predicate Schema]] |
| D17 | Never forget: no DELETE on triples or terms | [[time-model#Never Forget]] |
| Q20 | High-churn state goes in a `volatile` side table, not the graph | [[storage#Volatile Table]] |
| D18 | Speculative `with` via SQLite SAVEPOINT; no branches | [[time-model#Speculative Transactions]] |

## Open Inputs

Two facts are still unknown. They affect priorities, not the design.

- **Scale ceiling:** triples per database and writes per second. This sets benchmark targets and decides whether LFTJ is ever built. See [[roadmap#Benchmarks]].
- **First consumer:** MCP server, Python (PyO3), TS/Node, or WASM. This decides which binding ships first. See [[api#Bindings]].
