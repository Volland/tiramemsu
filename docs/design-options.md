# Tiramemsu: design options for a layered, time-travelling graph DB on SQLite

Status: draft for decision · 2026-09-29

**Goal:** a graph database on top of SQLite with:
- **addressable triples**: every statement has an identity, so it can be the subject or object of other statements;
- **Datomic-style time travel**: immutable facts, transactions as entities, and `as-of`, `since`, `history` and `with` queries;
- **two query dialects over one store**: SPARQL and Cypher;
- an internal architecture modelled on **MillenniumDB**.

This document has three parts:
1. The prior art, briefly: what each system teaches.
2. A **decision tree**. Each decision lists its options with trade-offs and a recommendation (★).
3. A reference architecture that follows the ★ path, with an MVP cut and open questions.

> The shared Claude conversation (claude.ai/share/6db0bcd4…) could not be read. Cloudflare blocks automated fetches, and the page renders client-side. Anything decided there is not reflected here yet.

---

## 1. Prior art at a glance

| System | Data model | Statement identity | Time | Storage | Languages | Status / licence |
|---|---|---|---|---|---|---|
| **MillenniumDB** | Domain graph `(s,type,t,eid)` + Labels + Properties | **Token** (edge id is an object) | None user-visible (page versioning for MVCC) | Custom B+trees, 4 KB pages | SPARQL 1.1, MQL (Cypher-like), GQL (early) | Active, GPL-2.0, "not production ready" |
| **Datomic** | Datoms `[e a v tx op]` | None per datom; tx is an entity | Tx-time only: `as-of`, `since`, `history`, `with` | LSM-like immutable segments | Datalog | Closed source, free binaries |
| **Fluree** | Flakes `(g,s,p,o,dt,t,op)` | Value-based EdgeKey + RDF 1.2 reifiers | Time travel + branches; **history sidecar** per leaf | Content-addressed leaves | SPARQL, JSON-LD query, **openCypher** | Active, **BUSL-1.1** |
| **XTDB v2** | Bitemporal SQL tables | Row | **Bitemporal** (valid + system) | Arrow columnar, LSM on object store | SQL, XTQL | Active, MPL-2.0 |
| **CozoDB** | Datalog relations | Row | Opt-in per relation: `Validity` key column, **skip-scan as-of** | **SQLite**, RocksDB, sled | CozoScript (Datalog) | Dormant, MPL-2.0 |
| **TerminusDB** | RDF-ish documents | None | Git-like **immutable delta layers** + squash | Succinct structures | WOQL, GraphQL | Active, Apache-2.0 |
| **Oxigraph** | RDF 1.2 | Triple terms by value | None | RocksDB, 11 column families, hash term ids | SPARQL 1.1/1.2 | Active, Apache/MIT |
| **Jena TDB2** | RDF quads | None | None (COW MVCC + compaction) | Custom B+trees | SPARQL | Active, Apache-2.0 |
| **Apache AGE** | LPG on Postgres | 64-bit tagged `graphid` | None | PG tables per label | Cypher → PG query tree | Active, Apache-2.0 |
| **Kùzu → LadybugDB** | Columnar LPG | Internal rel ids | None | Columnar + CSR | openCypher | Kùzu archived 10/2025; LadybugDB active, MIT |
| **Graphiti / Zep** | Temporal KG for agents | Edges are id'd "facts" | **Bitemporal edges** (`valid_at/invalid_at`, `created_at/expired_at`) | Neo4j, FalkorDB, Kuzu | Python API | Active, Apache-2.0 |
| **Neptune OneGraph (1G)** | Everything is a statement with id | Token | None | Poseidon engine | openCypher + SPARQL | Commercial |
| **AtomSpace** | Typed metagraph | Content-addressed (type + outgoing set) | None | RocksDB etc. | Atomese | Slow, AGPL |
| **Datahike / Datalevin / Datascript** | Datoms | None | Datahike: yes; others: no | konserve / LMDB / memory | Datalog | Active, EPL |
| **simple-graph / sqlite-graph** | Nodes + edges on SQLite | None / immature | None | SQLite, recursive CTEs | Cypher function (sqlite-graph) | Dormant / alpha |

**What to take from each:**
- **MillenniumDB:** ObjectId encoding, permutation indexes, worst-case-optimal (LFTJ) joins, path operators.
- **Datomic:** the transaction-as-entity model and the `as-of`/`since`/`history`/`with` API.
- **Fluree:** keeping history out of the hot indexes.
- **CozoDB:** the as-of skip-scan on SQLite.
- **Graphiti and XTDB:** valid time.
- **OneGraph:** unifying RDF and LPG.
- **AGE:** Cypher→SQL translation plus a native variable-length path operator.
- **Oxigraph and TDB2:** term dictionary and inlining.

**What to avoid:**
- simple-graph: edges without ids, and recursive-CTE traversal.
- AGE: property blobs that can't be indexed per key.
- rdflib-sqlalchemy: evaluating joins in the host language over single-pattern lookups.

**Lesson from Kùzu, Cozo and cr-sqlite:** single-sponsor embedded databases stall. Keep the core small and tied to the SQLite file format.

**Standards status (Sept 2026):**
- RDF 1.2 is a Candidate Recommendation. Triple terms are allowed **only in object position**, and an asserted statement gets its identity through a **reifier** (`_:r rdf:reifies <<( s p o )>>`).
- SPARQL 1.2 is a Working Draft.
- Cypher 25 defaults to `DIFFERENT RELATIONSHIPS`, i.e. relationship isomorphism. GQL is an ISO standard.

---

## 2. Decision tree

The order of the decisions matters: later decisions depend on earlier ones. D1–D3 are the roots.

```
D1 Scope ──► D2 Language/packaging
   │
   └► D3 Meaning of "layered" ──► D4 Statement model ──► D5 Statement identity ──► D6 Nesting
                                        │
                                        └► D7 Time model ──► D8 Tx semantics ──► D9 History storage
                                                                                   │
D10 ObjectId encoding ──► D11 Index set ◄──────────────────────────────────────────┘
                              │
                              └► D12 Execution strategy ──► D13 Front-ends & semantics ──► D14 Time-travel syntax
                                                               └► D15 Paths
D16 Schema · D17 Forgetting/excision · D18 Branching & sync  (can be decided later)
```

---

### D1: Scope and workload

| Option | Scale | Fit with SQLite |
|---|---|---|
| **A ★ Embedded agent / personal memory** | 10⁴–10⁷ statements, one process, small frequent transactions | Excellent: one file, ACID, runs on WASM and mobile |
| B Local-first KG app with sync | 10⁶–10⁸, devices sync | Good; needs a replication story (D18) |
| C Server analytical graph DB | 10⁹+, many writers | Poor. MillenniumDB needs 203 GB for 1.26 B triples on custom B+trees; SQLite would be worse |

**✅ DECIDED (2026-09-29): A — embedded agent / personal memory.** Every other decision below assumes this. Growth into B is kept possible but is not a v1 goal.

### D2: Host language and packaging

| Option | Pros | Cons |
|---|---|---|
| **Rust core ★**, shipped as a library + SQLite loadable extension + WASM build | Speed for the LFTJ and path executors; `rusqlite` gives access to prepared-statement seeks, virtual tables and custom functions; bindings for Python, JS and Swift | Longer to build |
| TypeScript on `better-sqlite3` / `wa-sqlite` | Fast iteration; browser-native | Executor overhead in JS; per-seek statement cost dominates LFTJ |
| Python on `sqlite3` | Fastest prototype | Too slow for WCO joins and paths; fine only as a reference implementation |
| C extension using SQLite's internal B-tree API | Fastest seeks | Fragile; the internal API is unsupported |

**✅ DECIDED (2026-09-29): Rust core** on `rusqlite` (bundled SQLite). Assumed default until you say otherwise: M0 is built directly in Rust, with no Python prototype. Bindings (PyO3, napi-rs, WASM, MCP server, loadable extension) are prioritised once the first consumer is known.

### D3: What does "layered" mean? (needs your answer)

The word covers four different ideas, and they lead to different designs:

| Option | Meaning | Precedent | Implication |
|---|---|---|---|
| L1 | **Architectural layers**: storage → index → algebra → dialects | MillenniumDB | Always true; not a feature by itself |
| L2 | **Multilayer graph**: statements can be about statements, to any depth (metagraph) | MillenniumDB quad model, 1G, AtomSpace | Drives D5/D6 |
| L3 | **Named graphs / contexts** as layers (per agent, per source, per session) | RDF datasets, Fluree ledgers | Adds a `g` column to every statement and every index |
| L4 | **Immutable delta layers** stacked per commit, with overlays for branches or speculation | TerminusDB, Datomic `with`, Dolt | Drives D9 and D18 |

**✅ DECIDED (2026-09-29): L2. "Layered" means layers are built *on top of* statement ids.** A triple has an id (MillenniumDB-style edge id); properties attach to that id; further layers (provenance, confidence, valid time, context/graph membership, beliefs about facts) are built by attaching data to ids from lower layers.

Consequences:
- **No `g` column in the core key (provisional).** Context or graph membership becomes an upper-layer annotation on the eid, not a fixed quad position. Trade-off: graph-scoped queries become a join rather than a key prefix. Revisit if per-agent isolation turns out to need physical partitioning; a separate SQLite file per tenant is a cheap alternative.
- L4 stays limited to temporary overlays for `with` (D8).

### D4: Core statement model

| Option | Shape | Pros | Cons |
|---|---|---|---|
| a. MillenniumDB split | `Edge(s,p,o,eid)` + `Label(n,l)` + `Prop(obj,key,val)` | Fast property lookups; Cypher never traverses properties by accident | Three relations, each needing its own time travel and provenance; SPARQL has to union them |
| **b. 1G uniform statements ★** | `Stmt(eid, s, p, o, g)`; properties are statements whose `o` is a literal; labels are `rdf:type`-like statements | One relation carries identity, time, provenance and reification. Matches a datom `[e a v tx]` + eid | Cypher must separate "edge statements" (object is a node) from "property statements" (object is a literal) through the ObjectId kind |
| c. Datomic EAV, no statement id | `[e a v tx op]` | Simple, proven | Fails the "addressable triples" requirement |
| d. Content-addressed hyperedges | id = hash(type, outgoing set) | Elegant n-ary edges; natural dedup | A token like "the same edge twice" can't be expressed; LPG parallel edges break |

**✅ DECIDED (2026-09-29): b, uniform statements.** Properties are statements with their own eids, so a property can itself carry properties to any depth. MillenniumDB's separate Properties table is dropped; its ObjectId, permutations, LFTJ and path engine are kept. Assumed default for the Cypher view: a literal-valued statement is a property, a node- or eid-valued statement is a relationship, and a per-predicate `isEdge` schema flag overrides this.

Original rationale: every statement is a row with an id. "Is this an LPG edge or a property?" follows from the object's kind (node vs literal), with an optional per-predicate override in the schema (D16). N-ary relations are modelled TypeDB-style: a node carries role statements.

### D5: Statement identity semantics (token vs type)

| Option | Meaning | Effect |
|---|---|---|
| a. Type: `eid = hash(s,p,o,g)` | The same triple is always the same statement | Pure RDF set semantics; **LPG parallel edges impossible**; retract + re-assert reuses the id |
| b. Token: fresh `eid` per assertion | Each assertion is a distinct occurrence | LPG-native; parallel edges allowed; RDF must deduplicate on projection |
| **c. Hybrid ★ (RDF 1.2 reading)** | `(s,p,o)` is the triple *term* (value); `eid` is a *reifier/occurrence* (token) | SPARQL's graph view is the set of distinct `(s,p,o,g)` over live statements. Cypher sees each `eid` as a relationship. `<<( s p o )>>` refers to the value; `~eid` / reifier refers to the occurrence |

Sub-decisions:
- **Does an eid survive retract then re-assert?** ★ No. Each assertion episode gets a new eid. Consequence: **an eid is immutable, and its lifetime is a single interval `[tx_add, tx_ret)`**. That makes D9 much simpler.
- **What if the same live `(s,p,o,g)` is asserted twice?**
  - ★ Default to idempotent (return the existing eid), which is RDF-friendly.
  - Allow a per-predicate or per-call `multi` flag for LPG parallel edges.
- **Updating an LPG edge's property** does not change the edge's eid. The property is its own statement about `eid`.

**✅ DECIDED (2026-09-29): c, idempotent by default with an explicit "new".**
- `assert` (SPARQL `INSERT`, Cypher `MERGE`) returns the existing live eid for the same `(s,p,o)`.
- `create` (Cypher `CREATE`) always mints a new eid.

**Added requirement:** every change is recorded as a time-travel event carrying a **timestamp and an operation** (Datomic datom style `[eid s p o tx op]`, where tx has a wall-clock instant). See D9 for the log design.

**Decided with D9e:** eids are never reused; statement content is immutable; an update is retract + new eid.

### D6: Nesting (statements about statements)

| Option | Rule |
|---|---|
| a. RDF 1.2 strict | Triple terms only as objects |
| **b. Metagraph ★** | `eid` is an ordinary ObjectId, allowed in the `s`, `o` (and even `p`) position of other statements, to any depth |

**★ b.** It is a superset of RDF 1.2: export maps `eid` to a reifier node, so it stays standards-compatible on the wire. Cycles (a statement about itself) are either forbidden at write time or left unrestricted. ★ Forbid direct self-reference only.

### D7: Time model

| Option | Axes | Precedent | Cost |
|---|---|---|---|
| a. Tx-time only | "When did the DB learn it?" | Datomic | Simplest; the most-cited Datomic limitation |
| b. Full bitemporal in the core | Tx time + valid time on every row, both indexed | XTDB v2, Graphiti | 2D interval queries; heavier indexes and more complex semantics (corrections of the past) |
| **c. Tx-time native, valid time as statements about eid ★** | `tx_add/tx_ret` columns; valid time = `(eid, :validFrom, t)` / `(eid, :validTo, t)` statements, promoted to an indexed side table if needed | Unique to addressable triples: valid time is simply metadata on a statement | Valid-time queries go through property joins until promoted |

**✅ DECIDED (2026-09-29): valid-time columns on the triple.** The option labels here differ from the interview: this is the "columns `v_from`/`v_to` on `triple`" option, closest to row b above, and it overrides the earlier ★. Schema change: `triple(eid, s, p, o, t_add, t_ret, v_from, v_to)`, where NULL means unbounded.
- **✅ Corrections (Q9, decided): c1. Valid time is immutable content.** A correction is retract + a new eid, so `(s,p,o,v_from,v_to)` never changes for an eid and `triple` rows stay fully immutable apart from `t_ret`.
  - Consequence: under D8 (cascade in both positions), a correction retracts every annotation and reference of the old eid. How to carry them over is open (Q10).
- **✅ Carry-over (Q10, decided): b. The `supersede` operation, cascade and replay.**
  1. Compute the cascade set C of the old eid.
  2. Retract all of C.
  3. Re-assert every triple in C with an id-substitution map `{e1→e10, e2→e11, …}` and the corrected content.
  4. Record `(e10 sys:supersedes e1)`.

  All in one tx, bounded by `maxCascade`. `supersede` is the general **update verb**: date corrections, closing an interval (`v_to`), and content fixes.
- **Idempotency with valid time (assumed, not yet confirmed):** `assert` returns the existing eid for a live triple with the same `(s,p,o)` and an **overlapping** valid interval. A non-overlapping interval is a new episode with a new eid.
- Benefit: two episodes of the same `(s,p,o)` (Alice at Acme 2020–22 and again 2024–) can be live at once as distinct eids.

Original rationale for statements-about-eids: agent memory needs valid time: Graphiti invalidates contradicted facts by closing their validity window. Addressable statements let you express it **without new core machinery**. Reserve the IRIs and write a `validAt(t)` operator on day one; add a dedicated interval index later.

### D8: Transaction semantics

| Topic | Options | ★ Recommendation |
|---|---|---|
| Tx id | Wall clock / monotonic int / hybrid logical clock | **Monotonic int `t`**. `txInstant` is a statement on the tx entity. HLC only if B (sync) happens |
| Tx as entity | Yes / no | **Yes.** The tx id is an ObjectId; author, source, prompt, model and reason are ordinary statements about it |
| Cardinality | Schema-free / Datomic `one` vs `many` | **Per predicate, default `many`**. `one` produces an implicit retraction of the previous live value in the same tx |
| Retraction | Explicit / by pattern / cascade | Explicit by eid or by `(s,p,o)`. ★ **Retracting a statement cascades to statements *about* it** (nested annotations), as Fluree does. Make this configurable |
| `with` | Speculative apply | Apply to an in-memory overlay (L4); never persisted |
| Isolation | SQLite WAL, single writer | Readers take a SQLite snapshot. Historical reads are always consistent because history is immutable |

**✅ DECIDED (2026-09-29): cascade in both positions (option c).** Retracting eid `e` retracts, in the same tx and recursively, every live triple with `s = e` **or** `o = e`.
- **Only statement ids cascade.** Nodes and IRIs have no lifetime, so they never trigger a cascade; retracting `(alice worksAt acme)` does not touch other triples about `alice` or `acme`.
- **Nothing is lost.** Cascaded rows get the same `t_ret`, so `asOf(t_ret − 1)` and `history()` still show the full reasoning (e.g. "what did belief9 rely on before tx 205?").
- **Retraction reasons live on the tx** (`tx205 :reason "user correction"`), never on the retracted triple: a triple about `e1` added in the same tx would be cascaded away at once.
- **Guard rails (proposed):**
  - The tx result returns the full cascade set.
  - A `dryRun` option previews it.
  - A configurable `maxCascade` limit aborts the tx if exceeded. Object-position cascades can chain through belief networks.
- Implementation: a recursive walk over `live_spo` (`s = e`) and `live_osp` (`o = e`), with a visited set to handle cycles.

### D9: How history is stored

Because of D5c, each eid has exactly one interval, so the problem is "interval rows plus efficient as-of".

| Option | Layout | As-of cost | Current-state cost | Notes |
|---|---|---|---|---|
| a. One table, interval predicate | `stmt(eid,s,p,o,g,tx_add,tx_ret)` + permutation indexes containing `tx_*` | Range scan with a filter; degrades with churn | Every index carries dead rows | Naive; Cozo measured about 20× slowdown |
| b. Partial indexes | Same table; `CREATE INDEX … WHERE tx_ret IS NULL` for current-state permutations | Via the full table | Fast (only live rows indexed) | SQLite only uses a partial index when the WHERE clause **repeats the predicate verbatim**; the generated SQL must do that |
| **c. Current + history split ★** | `live_*` permutations (WITHOUT ROWID, live rows only) + `hist_*` permutations keyed `(s,p,o,g, tx_add DESC, eid)` holding `tx_ret` | **Cozo skip-scan**: per key prefix, seek to the first `tx_add ≤ t`, check `tx_ret > t`, jump to the next prefix | Same as a DB without history | Fluree sidecar / XTDB pattern. Writes touch both sets; retraction moves the row from live to history |
| d. Delta layers per tx | TerminusDB-style additions/removals per commit + squash | O(#layers) without squash | Needs squash | Good for branching; poor fit for SQLite pages |

~~★ c for the core~~. **Superseded, see below.**

**✅ DECIDED (2026-09-29): e. Combined triple with its own lifetime.** This is b, made cheap by the rule that ids are never reused.

- **Layout:** one `triple(eid, s, p, o, t_add, t_ret)` table.
  - Live permutations are **partial indexes** `WHERE t_ret IS NULL`, with `t_ret` in the key so they stay covering.
  - History permutations are full indexes `(…, t_add DESC, t_ret)`.
  - The log is a view over `t_add` / `t_ret`.
- **Current-state cost:** a covering partial index; dead rows are never touched.
- **As-of cost:** a covering `hist_*` index seek.
- **Maintenance:** SQLite keeps every index in sync, so there is no hand-written live↔hist code. Retract is one `UPDATE t_ret`.
- Verified with `EXPLAIN QUERY PLAN` on SQLite 3.53.

Rules this relies on (all decided):
- **Ids are never reused.** Content `(s,p,o)` is immutable, so each eid has exactly one assert (`t_add`) and at most one retract (`t_ret`). Operation + timestamp come from the row, and `event(t, eid, op)` is a view.
- **An update is retract + a new eid.** Annotations on the old eid stay with the old value in history.
- **`confirm`** (corroboration of an already-live fact) is a layer triple `(eid, sys:confirmedBy, tx)`, not a special op.
- ~~`noHistory` predicates~~ and ~~excision~~: **removed by D17 (never forget)**. `triple` rows are never deleted.
- Generated SQL must repeat `t_ret IS NULL` verbatim for SQLite to pick the partial index.

When to revisit: if ids ever need to be re-opened after retraction, switch to Datomic-flat rows `(eid,s,p,o,t,op)`. d's idea (delta layers) stays reserved for `with` overlays (D18).

### D10: ObjectId encoding

MillenniumDB puts a type tag in the **high byte** of a 64-bit id and assumes **unsigned** ordering. In SQLite this is wrong twice over:
- SQLite compares integers as **signed**, so tags ≥ 0x80 sort first.
- SQLite stores small integers in 1–4 bytes. A high tag forces 8 bytes everywhere.

| Option | Pros | Cons |
|---|---|---|
| a. High-byte tag (MillenniumDB) | Groups by type; range scans per type | Always 8 bytes; must avoid the sign bit |
| **b. Low-bit tag ★** (`id = payload << 4 \| tag`) | Small varints for dense ids; still one INTEGER column; inline ints keep their order within a tag | Kinds interleave in the index, so range filters must check the tag |
| c. Separate `kind` column | Clearest | Widens every permutation key by one column |
| d. 128-bit content hash (Oxigraph) | No dictionary round trip on write; deterministic across replicas | 16-byte BLOB keys, about 2× index size |

**★ b.** Tags: node/IRI-dict, blank, statement (eid), tx, inline-int, inline-bool, inline-date, dict-string, dict-typed-literal, dict-langstring, inline-short-string (≤ 7 bytes, as MillenniumDB does).
- Dictionary: `term(id INTEGER PRIMARY KEY, lex TEXT, dt INTEGER, lang TEXT)` with a unique index.
- Consider (d)-style hashes only for IRIs if B (sync) needs globally stable ids.

**✅ DECIDED (2026-09-29): a. Low-bit tag**, `id = payload << 4 | tag`.
- Inline tags: `INT` (60-bit signed), `BOOL`, `DATE`/`DATETIME` (epoch ms), `SHORT_STR` (≤ 7 bytes), `STMT`, `TX`, `NODE`, `BNODE`.
- Dictionary tags: `IRI`, `STR`, `LANG_STR`, `TYPED`, `DOUBLE`/`DECIMAL`.
- Range scans on a predicate seek from `(v<<4)|TAG` and filter on `o & 15 = TAG`.

Assumed defaults (not yet confirmed):
- Doubles and decimals go to the dictionary, with an indexed `num REAL` column in `term`. Fixed-point `INT` is documented as an option for hot predicates.
- Cypher nodes and RDF IRIs share one id space. Anonymous LPG nodes get `NODE` ids, exported as skolem IRIs `urn:tiramemsu:node:<n>`.

### D11: Index set

> **Updated by D9e:** the permutations below are now **indexes on the single `triple` table**: partial (`WHERE t_ret IS NULL`) for current state, full with `t_add DESC` for history. They are no longer separate `WITHOUT ROWID` tables, and there is no `g` column (D3). See the schema sketch in §3.

Original analysis. For `Stmt(eid,s,p,o,g)`, current state, each as a `WITHOUT ROWID` covering table:

| Permutation | Serves | ★ |
|---|---|---|
| `eid → s,p,o,g,tx_add` (rowid PK) | Statement lookup, annotations, provenance | Required |
| `SPO(g,eid)` | Outgoing edges and properties of a node | Required |
| `POS(g,eid)` | Predicate + value lookup (property index, reverse lookup by value) | Required |
| `OSP(g,eid)` | Incoming edges | Required |
| `PSO` | Predicate scans in joins | Optional |
| `G***` variants | Graph-scoped queries | Only if named graphs are heavily used (L3) |
| Equality trees (MillenniumDB `?x ?x ?y`) | Self-loops | Skip; filter instead |

History: `hist_spo`, `hist_pos` keyed `(…, tx_add DESC, eid)` plus a `tx → eids` log index (Datomic's Log) for `since` and replay.
- Budget: about 2–3× raw size, as in MillenniumDB and Jena LF.
- Make the set **configurable**, as Quadstore does.

### D12: Execution strategy

| Option | How | Pros | Cons |
|---|---|---|---|
| a. Everything to SQL | Compile SPARQL/Cypher → SQL over the permutation tables (AGE approach) | Reuses SQLite's planner; simple | Nested-loop only, so cyclic patterns (triangles) blow up; paths as recursive CTEs can't do trail or shortest semantics well |
| b. Own executor only | Volcano/LFTJ operators in Rust; each trie iterator = prepared seek `SELECT … WHERE k1=? AND k2=? AND k3>=? ORDER BY k3 LIMIT 1` | MillenniumDB-faithful, with WCO joins | Per-seek statement overhead; you write a whole planner |
| **c. Hybrid ★** | One logical algebra; acyclic BGPs, filters and aggregates → SQL; cyclic BGPs → LFTJ operator; paths → native operator; SQLite **virtual tables** expose operators back into SQL | Best of both; incremental | Two code paths |

**✅ DECIDED (2026-09-29): c. Hybrid.** One logical algebra, with the physical planner routing by shape:
- acyclic patterns, filters and aggregates → generated SQL;
- paths → a native Rust operator, exposed as a SQL table-valued function `tm_path(...)`;
- cyclic patterns → LFTJ.

A single `scan(triple, view)` codegen function owns all temporal predicates (now, asOf, validAt, history).

Assumed (not yet confirmed): LFTJ is deferred to M4, behind a triangle benchmark.

Original note: start with (a) for everything except paths, then add LFTJ when cyclic benchmarks demand it. Batch LFTJ seeks as range reads and gallop in memory to amortise statement overhead.

### D13: Front-ends and semantic mismatches

**Common IR:** a relational/graph algebra (BGP, Join, LeftJoin, Filter, Union, Project, Distinct, Aggregate, Path, Temporal). Each operator carries **semantic flags**:

| Flag | SPARQL | Cypher |
|---|---|---|
| Graph data | Set of `(s,p,o,g)`, deduplicated over eids | Bag of relationships (eids) |
| Pattern matching | Homomorphism | **Relationship isomorphism** (`eid_i ≠ eid_j` constraint), `REPEATABLE ELEMENTS` opt-out |
| Missing values | Unbound; FILTER errors → false | NULL, three-valued logic |
| Optional | `OPTIONAL {}` (left join, non-well-designed quirks) | `OPTIONAL MATCH` |
| Upsert | none (INSERT/DELETE WHERE) | `MERGE` → unique-key upsert inside the single-writer tx |
| Blank nodes | Local | n/a; skolemise when exposing ids |
| Paths | Property paths: endpoints only, set semantics | Returns paths: walk/trail/simple/shortest |

**Vocabulary mapping (the OneGraph problem):**
- LPG label ↔ `rdf:type <vocab:Label>`.
- LPG key/type ↔ IRI via a configurable `@vocab` base (as in Fluree).
- LPG relationship ↔ statement eid whose object is a node.
- LPG property ↔ statement whose object is a literal.
- Property on a relationship ↔ statement with `s = eid`.

**✅ DECIDED (2026-09-29): vocabulary mapping (Q15, option a). One configurable `@vocab` base + a prefix table.**
- A bare Cypher name maps to `@vocab` + name (verbatim). Backticked CURIEs resolve through the prefix table.
- IRI → Cypher: a bare local name inside `@vocab`, otherwise a CURIE, otherwise a backticked full IRI.
- Labels ↔ `rdf:type`.
- The prefix table is stored as `sys:prefix` triples, so it is versioned.

Assumed (not yet confirmed):
- the default base is `urn:tiramemsu:v:`;
- the `sys:` namespace is reserved and hidden from `labels()` / `keys()` / `properties()` by default.

**Scope options:**

| Option | ★ |
|---|---|
| SPARQL 1.1 query + update, and SPARQL 1.2 triple terms/reifiers/annotations `{| |}` | ★ 1.1 core first, 1.2 annotations next (they map directly onto eids) |
| openCypher (read + CREATE/MERGE/SET/DELETE) vs full GQL | ★ openCypher subset first, aligned with Cypher 25 / GQL match modes |
| Datalog (Datomic-style) as a third dialect | Optional: it is the most natural surface for time travel and would be cheap on this model |

**✅ DECIDED (2026-09-29): both dialects in parallel (option c).** SPARQL via `spargebra`. Cypher via an existing Rust openCypher parser, to be evaluated (`opencypher`, `decypher`, `open-cypher`, `cypher_parser`), with a hand-written `chumsky` subset as the fallback.

Mitigation for the risk of two half-finished front ends:
- the shared IR is built first;
- a **differential test suite** runs equivalent SPARQL and Cypher queries on the same data and asserts identical results.

Assumed (not yet confirmed): the v1 Cypher and SPARQL subsets listed above; no Datalog in v1.

**✅ DECIDED (2026-09-29): the Cypher dual view (Q14, option b). An eid is both a relationship and a node.**
- A relationship variable may also appear in node position, e.g. `MATCH (a)-[r:WORKS_AT]->(c), (b:Belief)-[:SUPPORTED_BY]->(r)`.
- Such nodes carry the implicit label `:Statement` (assumed name) and expose temporal metadata (`txAdded`, `txRetracted`, `validFrom`, `validTo`).
- `startNode`, `endNode` and `type` still work.
- Our Cypher is a documented superset; every standard query is unchanged.
- SPARQL `~ ?r` and Cypher `(r)` bind the same eid, so the differential tests cover layers.

### D14: Time-travel surface syntax

| Option | Example | ★ |
|---|---|---|
| **API-level database value** | `db.asOf(1042).sparql(q)`, `db.history().cypher(q)`, `db.with(txData)` | ★ Primary (Datomic style; composable) |
| SPARQL dataset pseudo-graphs | `FROM <tx:1042>` · `FROM <tx:since/1000>` · `FROM <tx:history>` | ★ Sugar |
| Cypher `USE` clause | `USE memory AS OF 1042 MATCH …` | ★ Sugar (Cypher 25 / GQL has `USE`) |
| Temporal functions on statements | Cypher `txAdded(r)`, `txRetracted(r)`, `validAt(r, t)`; SPARQL `tm:txAdded(?r)` over reifiers | ★ Needed for "when did this change?" |
| Tx metadata queries | `MATCH (tx:Tx {author:'agent-7'})-[:asserted]->(r)` via virtual predicates `tm:assertedIn` | ★ Makes history queryable as a graph |

**✅ DECIDED (2026-09-29): b. API + in-query syntax in both dialects, scoped per pattern.**

| | SPARQL (standard grammar, magic IRIs) | Cypher (superset) |
|---|---|---|
| As of tx / wall clock | `FROM <tm:asOf/150>` · `FROM <tm:asOf/2026-09-01T12:00Z>` | `USE AS OF 150` · `USE AS OF datetime(…)` |
| Valid time | `FROM <tm:validAt/2025-03-01>` | `USE VALID AT date(…)` |
| History | `FROM <tm:history>` | `USE HISTORY` |
| Per-pattern (diffs) | `GRAPH <tm:asOf/150> { … }` | `CALL { USE AS OF 150 MATCH … RETURN … }` |
| Statement metadata | `?r tm:txAdded ?t`, `tm:validFrom` | `r.txAdded`, `r.validFrom` |

- **The view is per pattern in the IR**, because every `scan(triple, view)` already carries one. That makes per-pattern scoping nearly free, so it ships in v1.
- Cypher v1 therefore adds **`CALL { … }` subqueries** (uncorrelated plus importing `WITH`), overriding the earlier "no CALL in v1".
- **Defaults:** no time clause = tx now, valid time unfiltered. Valid-time filtering is always opt-in.
- API: `db.asOf(t|instant).validAt(d).history()` returns a view object; the in-query syntax lowers to the same object.

### D15: Path queries

- A native BFS/DFS over the product of the query automaton and the graph, as in MillenniumDB. It must never be a recursive CTE for trail, simple or shortest modes.
- Modes: `ANY`, `ALL SHORTEST`, `SHORTEST k`, and `WALK / TRAIL / SIMPLE / ACYCLIC`.
- Both endpoint-only evaluation (SPARQL) and path-returning evaluation (Cypher/GQL).
- Time-aware: the traversal reads through the same as-of view.
- ★ At least one endpoint must be bound in v1.

**✅ DECIDED (2026-09-29): paths are available in queries and in the API.**
- **Queries:** SPARQL property paths; Cypher `*min..max`, `shortestPath` and `allShortestPaths`.
- **API:** `db.path(start, pattern, mode, view)` and the SQL table-valued function `tm_path(...)`, all backed by the same native operator.

v1 modes:
- SPARQL reachability `* + ? / | ^`;
- Cypher trail with a default cap of 15 hops when unbounded;
- `ANY SHORTEST`, `ALL SHORTEST`;
- at least one endpoint bound.

Later: `SIMPLE`/`ACYCLIC`, `SHORTEST k`, time-respecting paths.

Virtual hops `sys:subject` / `sys:object` (and their reverses) are computed from the triple row, so paths can walk through statement layers.

Assumed (confirm): the mode list, the hop cap, and the virtual hops.

### D16–D18: can be decided later

**✅ D17 DECIDED (2026-09-29): NEVER FORGET.** There is no excision, TTL or physical deletion. Forgetting is expressed only through time and operation: `retract` sets `t_ret`, and history is kept forever.
- **Engine invariant:** no code path issues `DELETE` on `triple` or `term`. The only mutation of a row is setting `t_ret` once. This invariant is testable (grep plus property test).
- `asOf(t)` is exact for every `t`, forever.
- Escape hatch if a legal erasure requirement ever appears, without breaking the invariant: **crypto-shredding**. Sensitive literals are stored encrypted with a per-subject key, and deleting the key makes them unreadable while the log stays intact. Not planned; noted only.

**✅ D18 DECIDED (2026-09-29): b. Short-lived speculative `with` via SQLite `SAVEPOINT` + `ROLLBACK TO`.**
- `db.with(txData, |spec| spec.query(...))` runs the full engine (cascade, schema, supersede, cardinality-one), then rolls back.
- Nothing reaches history, so D17 is unaffected.
- It holds the single write lock for its duration, so it is meant for short speculation.
- No long-lived branches and no git-style branching.
- Sync is deferred: the `event` view is the future replication feed.

**✅ Q20 DECIDED (2026-09-29): `noHistory` is dropped; high-churn state lives outside the graph.**
- Volatile values (`lastSeen`, counters, "current focus", per-turn scores) go in a `volatile(s, key, value, updated_at)` side table.
- Queries see them as virtual properties (`n.lastSeen`), but they are **not triples**: no eid, no history, no layers, and ordinary upsert/delete is allowed there.
- Rule of thumb: *if you'd ever ask "why" or "as of when" about it, it's a triple; otherwise it's volatile state.*
- The D17 invariant (no DELETE on `triple`/`term`) stays absolute.

**✅ D16 DECIDED (2026-09-29): b. An optional predicate schema stored as `sys:` triples.**
- Flags: `sys:unique`, `sys:cardinality` (`sys:one` / `sys:many`), `sys:valueType`, `sys:isEdge`. (`sys:noHistory` is dropped: see Q20 below.)
- Unknown predicates default to cardinality many, no constraints, history on. The schema is versioned and queryable.

Assumed (confirm):
- **Cardinality-one replacement = retract (with D8 cascade) + assert, with no replay.** It is a new fact, unlike `supersede`, which is a correction.
- `sys:unique` is enforced over live triples by a `live_pos` lookup in the writer transaction.
- A schema change that current data violates is rejected, and the violating eids are reported.


| # | Topic | Options | ★ Lean |
|---|---|---|---|
| D16 | Schema | Schemaless / optional Datomic-like predicate schema (type, cardinality, unique, `noHistory`, `isEdge`) / SHACL | Optional predicate schema stored as statements about predicates; SHACL validation later |
| D17 (✅ decided below) | Forgetting | Retract only / `noHistory` / **excision** (physical removal including history) / TTL | Excision is required for agent memory and GDPR: delete from `hist_*` + `live_*` + dictionary GC, and record an excision tx |
| D18 | Branching and sync | None / overlay branches (L4) / tx-log shipping / CRDT (cr-sqlite style) | Overlays for `with` and what-if; tx-log shipping for B (a datom log replicates naturally); hash ids (D10d) if multi-writer |

---

## 3. Reference architecture (★ path)

```
┌──────────────────────── Dialects ────────────────────────┐
│  SPARQL 1.1/1.2 parser   openCypher parser   (Datalog)    │
└──────────────┬───────────────────┬───────────────────────┘
               ▼                   ▼
        ┌──── Logical algebra + semantic flags ────┐
        │  BGP · Join · LeftJoin · Path · Temporal │
        └──────────────────┬───────────────────────┘
                           ▼
        ┌──────────── Physical planner ────────────┐
        │ SQL codegen │ LFTJ operator │ Path op     │
        └──────┬──────────────┬───────────────┬────┘
               ▼              ▼               ▼
┌──────────── Temporal view layer (db value) ──────────────┐
│ now → live_* partial idx   as-of t → hist_* idx   with → overlay │
└──────────────────────────┬───────────────────────────────┘
                           ▼
┌──────────── SQLite file (WAL, STRICT) ────────────────────┐
│ term dict · tx · triple(eid,s,p,o,t_add,t_ret) + indexes │
└───────────────────────────────────────────────────────────┘
```

### Schema sketch

```sql
-- Term dictionary (IRIs, long strings, typed literals). Inline values never land here.
CREATE TABLE term (
  id   INTEGER PRIMARY KEY,          -- payload; ObjectId = id<<4 | tag
  lex  TEXT NOT NULL,
  dt   INTEGER,                      -- datatype term id
  lang TEXT,
  num  REAL                          -- numeric value for DOUBLE/DECIMAL range queries
) STRICT;
CREATE UNIQUE INDEX term_lex ON term(lex, dt, lang);
CREATE INDEX term_num ON term(num) WHERE num IS NOT NULL;

-- Transactions are entities; their metadata (author, source, reason) are triples with s = tx ObjectId.
CREATE TABLE tx (
  t       INTEGER PRIMARY KEY,       -- monotonic
  instant INTEGER NOT NULL           -- wall clock ms UTC, forced strictly increasing
) STRICT;
CREATE UNIQUE INDEX tx_instant ON tx(instant);

-- Every triple ever asserted, carrying its own lifetime (D9e).
-- eid is never reused; (s,p,o) never changes; t_add = assert op, t_ret = retract op.
CREATE TABLE triple (
  eid   INTEGER PRIMARY KEY,         -- statement ObjectId (tag = STMT)
  s INTEGER NOT NULL, p INTEGER NOT NULL, o INTEGER NOT NULL,
  t_add INTEGER NOT NULL,
  t_ret INTEGER,                     -- NULL = live
  v_from INTEGER,                    -- valid time (D7), NULL = unbounded; immutable
  v_to   INTEGER                     -- valid time (D7), NULL = unbounded; immutable
) STRICT;

-- Current state: partial covering indexes (t_ret in the key keeps them covering).
CREATE INDEX live_spo ON triple(s,p,o,t_ret) WHERE t_ret IS NULL;
CREATE INDEX live_pos ON triple(p,o,s,t_ret) WHERE t_ret IS NULL;
CREATE INDEX live_osp ON triple(o,s,p,t_ret) WHERE t_ret IS NULL;

-- Time travel: full indexes, newest first within a key.
CREATE INDEX hist_spo ON triple(s,p,o,t_add DESC,t_ret);
CREATE INDEX hist_pos ON triple(p,o,s,t_add DESC,t_ret);

-- Volatile state (Q20): NOT triples, with no eid, history or layers. Exposed as virtual properties.
CREATE TABLE volatile (
  s          INTEGER NOT NULL,       -- node ObjectId
  key        INTEGER NOT NULL,       -- predicate ObjectId
  value      INTEGER NOT NULL,       -- ObjectId
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (s, key)
) WITHOUT ROWID, STRICT;

-- Valid time: "true on date X" over current beliefs.
CREATE INDEX valid_p ON triple(p, v_from, v_to) WHERE t_ret IS NULL;

-- Datomic "Log": since / replay / sync. Supersede = a retract + assert pair in one tx,
-- linked by a (new sys:supersedes old) triple.
CREATE INDEX log_add ON triple(t_add);
CREATE INDEX log_ret ON triple(t_ret) WHERE t_ret IS NOT NULL;
CREATE VIEW event(t, eid, op) AS
  SELECT t_add, eid, 'assert'  FROM triple
  UNION ALL
  SELECT t_ret, eid, 'retract' FROM triple WHERE t_ret IS NOT NULL;
```

Queries (the `t_ret IS NULL` predicate must appear verbatim for the partial index to be used):

```sql
-- now
SELECT o, eid FROM triple WHERE s=:s AND p=:p AND t_ret IS NULL;
-- as of t
SELECT o, eid FROM triple WHERE s=:s AND p=:p AND t_add<=:t AND (t_ret IS NULL OR t_ret>:t);
-- valid on date d (current beliefs)
SELECT s, o, eid FROM triple WHERE p=:p AND t_ret IS NULL
  AND (v_from IS NULL OR v_from<=:d) AND (v_to IS NULL OR v_to>:d);
-- since t (the log)
SELECT * FROM event WHERE t > :t ORDER BY t;
```

This sketch was checked on SQLite 3.53: it loads cleanly, returns the right results on the worked example below, and all three queries use covering indexes.

### Worked example: agent memory with provenance and time

```
tx 101 (author=agent-7, source=chat#42):
  e1: (:alice :worksAt :acme)
  e2: (e1 :confidence 0.8)            ← statement about a statement
  e3: (e1 :validFrom 2025-01-01)      ← valid time as a statement (D7c)
tx 205 (author=agent-7, reason="user correction"):
  retract e1  → e2, e3 cascade (D8)
  e4: (:alice :worksAt :globex)
  e5: (e4 :validFrom 2026-03-01)
```

- `db.asOf(150).cypher("MATCH (a {id:'alice'})-[r:worksAt]->(c) RETURN c, r.confidence")` returns acme, 0.8.
- `db.history().sparql("SELECT ?c ?t WHERE { :alice :worksAt ?c ~ ?r . ?r tm:txAdded ?t }")` returns acme 101 and globex 205.

---

## 4. Suggested MVP cut

1. **M0, model core in Rust** (about 1–2 weeks): `term`, `tx`, `triple` + indexes, and the tx API: `assert` (idempotent), `create`, `retract`, cascade, cardinality-one, `confirm` as a layer triple. As-of via `hist_*`. A property-based test that `asOf(t)` equals a replay of the log up to t.
2. **M1, SPARQL 1.1 BGP + FILTER + OPTIONAL → SQL**, plus `FROM <tx:…>` time travel and reifier/annotation syntax for eids.
3. **M2, openCypher MATCH/CREATE/SET/DELETE/MERGE → the same algebra**, with the relationship-isomorphism flag and the LPG↔RDF vocab mapping.
4. **M3, native path operator** (BFS, ANY/ALL SHORTEST, trail/simple).
5. **M4, LFTJ operator** for cyclic BGPs, with benchmarks against SQL-only.
6. **M5, `with` overlays, tx-log sync.** (No excision: D17.)

Benchmarks to track from M0: a churn test (N updates per key; as-of throughput vs no-history, with Cozo's over 70% as the bar), triangle queries (SQL vs LFTJ), 2-hop and shortest-path latency at 10⁶ and 10⁷ statements, and index size vs raw data.

---

## 5. Open questions for you

1. ~~**D1:** Scenario A, B or C?~~ → **A** (decided). Still open: target size and write rate?
2. ~~**D2:** Host language?~~ → **Rust core** (decided). Still open: who is the first consumer (MCP, Python, TS, on-device)?
3. ~~**D3:** Which meaning(s) of "layered"?~~ → **L2**: layers are built on triple ids (decided). Named graphs are not a core key column (provisional).
4. ~~**D4:** Statement model?~~ → **b, uniform statements** (decided).
5. ~~**D5:** Idempotent re-assert?~~ → **c**, plus a time-travel event log with timestamp and operation (decided).
6. ~~**D9:** History storage?~~ → **e, combined triple with its own lifetime** `(eid,s,p,o,t_add,t_ret)`; ids are never reused; `confirm` is a layer triple (decided).
7. ~~**D7:** Valid time?~~ → **columns `v_from`/`v_to` on the triple** (decided). Corrections: **c1, immutable; a correction = retract + new eid** (decided). Carry-over: **b, `supersede` with cascade and replay** (decided).
8. ~~**D8:** Cascade on retract?~~ → **c, both subject and object positions**, recursive, statement ids only; retraction reasons live on the tx (decided).
9. ~~**D10:** ObjectId encoding?~~ → **a, low-bit tag** (decided).
10. ~~**D12:** Execution?~~ → **c, hybrid**: SQL + native path TVF + deferred LFTJ (decided).
11. ~~**D13:** Dialects?~~ → **both SPARQL and Cypher in parallel**, shared IR + differential tests (decided). Datalog: assumed no for v1.
12. ~~**D14:** Time syntax?~~ → **b, API + per-pattern syntax in both dialects**; defaults tx=now, valid unfiltered (decided).
13. ~~**D17:** Excision?~~ → **never forget**; only retract (time + op); no DELETE anywhere (decided).
14. ~~**D18:** Speculation/branches?~~ → **b, SAVEPOINT-based `with`**; no branches (decided).
15. What did the shared conversation decide? Paste its key points and this document will be reconciled with it.

---

## 6. Addendum (2026-09-29): comparison with oxilite and SQLite plan measurements

The design was compared with [oxilite](https://github.com/Volland/oxilite), a shipped SQLite-backed RDF/SPARQL/Cypher/Datalog database by the same author. The schema's query plans were also measured on SQLite 3.53. Four questions went back to the user, and all four recommendations were accepted. The resulting decisions are D19–D25 in `lat.md/overview.md`.

### What the measurements showed

- **Join order.** With bound parameters and no statistics, a 4-pattern BGP over 1.1 M statements started from a 500 k-row pattern and took 272 ms. With `ANALYZE` (STAT4, which rusqlite's bundled build enables) it took 1 ms and started from the 50-row pattern. SQLite with statistics chose well on every skewed shape tried. → **D19**: statistics are mandatory and automatic. An engine-forced order (oxilite's D4) is only a benchmark-gated fallback.
- **Footprint.** The full schema needs about 153 bytes per statement, with indexes at 5.3× the table and `hist_*` at 44 % of the file. Partial history indexes are a benchmark-gated candidate.
- **Index choice.** `t_ret` must stay in the `live_*` keys, or SQLite stops using them as covering.

### Questions and answers

| Question | Options | Chosen |
|---|---|---|
| Named graphs (D3) | keep D3 and move time syntax to `SERVICE` / add a `g` column / no change | **Keep D3, fix syntax** → D21 |
| Erasure (D17) | schedule crypto-shredding / add an excise op / no change | **Schedule crypto-shred** → D24 |
| Reuse of oxilite | reuse parts / ideas only / build on oxilite | **Reuse parts** → D23 |
| Executor abstraction | sync executor trait / rusqlite only | **Executor trait** → D22 |

**Also decided:** D20 keeps the `xsd:dateTime` offset inline, since agent memory needs local times and SPARQL `TZ` needs the offset. D25 plans a retrieval milestone (FTS5 and vectors).

**Where tiramemsu keeps its own path**, and why: statement identity (oxilite needs reifiers for edge properties); bitemporal time, which oxilite lacks; covering history indexes rather than a change log (to be verified with oxilite's `as-of-latency` benchmark); and dictionary ids rather than hashes (one embedded writer, small varints).

---

## Appendix: sources

- MillenniumDB: [arXiv 2111.01540](https://arxiv.org/abs/2111.01540) · [Data Intelligence 2023](https://direct.mit.edu/dint/article/5/3/560/117375/MillenniumDB-An-Open-Source-Graph-Database-System) · [SIGMOD'24 demo](https://aidanhogan.com/docs/millenniumdb-demo.pdf) · [repo](https://github.com/MillenniumDB/MillenniumDB) ([object_id.h](https://github.com/MillenniumDB/MillenniumDB/blob/dev/src/graph_models/object_id.h), [quad_model.h](https://github.com/MillenniumDB/MillenniumDB/blob/dev/src/graph_models/quad_model/quad_model.h)) · paths: [2204.11137](https://arxiv.org/abs/2204.11137), [2306.02194](https://arxiv.org/pdf/2306.02194)
- Datomic: [index model](https://docs.datomic.com/indexes/index-model.html) · [filters](https://docs.datomic.com/reference/filters.html) · [excision](https://docs.datomic.com/operation/excision.html) · [forking the past](https://blog.danieljanus.pl/datomic-forking-the-past/) · ["not the history you're looking for"](https://vvvvalvalval.github.io/posts/2017-07-08-Datomic-this-is-not-the-history-youre-looking-for.html) · [tonsky internals](https://tonsky.me/blog/unofficial-guide-to-datomic-internals/)
- Fluree: [index-format](https://github.com/fluree/db/blob/main/docs/design/index-format.md) · [time-travel](https://github.com/fluree/db/blob/main/docs/concepts/time-travel.md) · [edge-annotations](https://github.com/fluree/db/blob/main/docs/design/edge-annotations.md)
- XTDB v2: [launch post](https://xtdb.com/blog/launching-xtdb-v2) · CozoDB: [v0.4 time travel notes](https://docs.cozodb.org/en/latest/releases/v0.4.html) · TerminusDB: [succinct delta layers paper](https://assets.terminusdb.com/research/succinct-data-structures-and-delta-encoding.pdf)
- Oxigraph: [architecture](https://github.com/oxigraph/oxigraph/wiki/Architecture) · Jena: [TDB2](https://jena.apache.org/documentation/tdb2/) · Apache AGE: [graphid.h](https://github.com/apache/age/blob/master/src/include/utils/graphid.h)
- Kùzu forks: [Szárnyas](https://szarnyasg.org/posts/kuzu-forks/) · [LadybugDB](https://github.com/LadybugDB/ladybug)
- Graphiti / Zep: [arXiv 2501.13956](https://arxiv.org/abs/2501.13956) · OneGraph: [Semantic Web 14 (2023)](https://content.iospress.com/articles/semantic-web/sw223273), [Poseidon arXiv 2510.11166](https://arxiv.org/abs/2510.11166)
- Standards: [RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/) · [SPARQL 1.2 Query](https://www.w3.org/TR/sparql12-query/) · [Cypher 25 match modes](https://neo4j.com/docs/cypher-manual/25/patterns/match-modes/)
- WCO joins: [LFTJ (Veldhuizen)](https://arxiv.org/abs/1210.0481) · Freitag et al. VLDB 2020 · Free Join SIGMOD 2023
