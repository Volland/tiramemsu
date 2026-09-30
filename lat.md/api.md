# API

The `tiramemsu` facade crate exposes three handles: `Db` opens the file and runs transactions, `View` is an immutable time selection that runs queries, and `Tx` holds the write operations.

## Rust Surface

The signatures below are the contract that bindings wrap. Names are fixed; details such as generic bounds may change during implementation.

```rust
pub struct Db { /* writer + reader pool */ }

impl Db {
    pub fn open(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Db>;   // tm-rusqlite host
    pub fn open_with_host(host: impl Host, path: impl AsRef<Path>, opts: OpenOptions) -> Result<Db>;
    pub fn capabilities(&self) -> Capabilities;                             // declared by the host
    pub fn optimize(&self) -> Result<()>;                                   // full ANALYZE
    pub fn transact<F>(&self, opts: TxOptions, f: F) -> Result<TxReport>
        where F: FnOnce(&mut Tx) -> Result<()>;
    pub fn with<F, G, R>(&self, ops: F, query: G) -> Result<R>      // speculative
        where F: FnOnce(&mut Tx) -> Result<()>, G: FnOnce(&View) -> Result<R>;
    pub fn now(&self) -> View;
    pub fn as_of(&self, at: TimeRef) -> View;                     // TimeRef::Tx(t) | TimeRef::Instant(ms)
    pub fn history(&self) -> View;
    pub fn events_since(&self, t: u64) -> Result<Vec<Event>>;
    pub fn cypher_write(&self, opts: TxOptions, q: &str, params: &CypherParams) -> Result<CypherResult>; // one transaction; rows + TxReport
}

impl View {
    pub fn valid_at(self, epoch_ms: i64) -> View;
    pub fn sparql(&self, q: &str) -> Result<SparqlResult>;        // SELECT | ASK | CONSTRUCT | update (current view only)
    pub fn sparql_with(&self, q: &str, opts: &SparqlOptions) -> Result<SparqlResult>; // opts.provenance: per-row eids
    pub fn cypher(&self, q: &str, params: &CypherParams) -> Result<CypherResult>;   // read-only; a write clause is Unsupported
    pub fn path(&self, start: ObjectId, path: &str, mode: PathMode, max_hops: u32) -> Result<Vec<PathRow>>;
    pub fn path_with(&self, start: ObjectId, path: &str, args: &PathArgs) -> Result<Vec<PathRow>>; // + graphs, time_respecting
    pub fn triples(&self, s: Option<ObjectId>, p: Option<ObjectId>, o: Option<ObjectId>) -> Result<Vec<Triple>>;
    pub fn graphs(&self) -> Result<Vec<ObjectId>>;                             // graphs with a visible membership, or declared
    pub fn graph_members(&self, graph: ObjectId) -> Result<Vec<Eid>>;          // member statements in this view
    pub fn dependents(&self, eid: Eid) -> Result<Vec<Eid>>;                    // what stands on eid (the cascade set on now)
    pub fn bundle(&self, root: Eid) -> Result<Bundle>;                         // root, its dependents and their evidence
    pub fn values(&self, s: ObjectId, key: ObjectId) -> Result<Vec<ObjectId>>; // statements, else volatile (Now only)
    pub fn encode(&self, v: &Value) -> Result<Option<ObjectId>>;              // lookup only, never inserts
    pub fn decode(&self, id: ObjectId) -> Result<Value>;
    pub fn events_since(&self, t: u64) -> Result<Vec<Event>>;
}
```

`Tx` is the core write handle, re-exported by the facade. The `TxCypher` extension trait adds `cypher(q, params)`, which runs a Cypher query with reads and writes inside the caller's `transact` closure ([[crates/tiramemsu/src/cypher.rs#TxCypher]]); `Tx::set_vocab` and `Tx::set_prefix` change the vocabulary configuration ([[data-model#Vocabulary Mapping]]). Besides the operations of [[time-model#Operations]] it offers `assert_with` (with `OnExisting::Confirm`), `new_bnode`, `clear_volatile`, `encode`, `lookup`, `decode`, `schema`, `t` and `instant`. Positions take any `IntoObject`: an `ObjectId`, `Eid`, `TxId` or `Value`.

- Named graphs ([[data-model#Named Graphs]]): `Tx::add_to_graph(eid, graph, opts) -> (membership_eid, is_new)` (idempotent, `opts.valid` bounds the membership), `remove_from_graph(eid, graph) -> bool`, `clear_graph(graph) -> Vec<Eid>`, `create_graph(graph)`, and the helpers `drop_graph`, `graph_declared`, `graph_has_members` and `live_graphs`. All reject a non-node graph with `InvalidGraphName`. `TxReport` lists new memberships in `memberships` and retracted ones in `memberships_retracted`, and no longer in `asserted` and `retracted`.
- Fact bundles ([[data-model#Fact Bundles]]): `View::bundle(root) -> Bundle` and `Tx::import_bundle(&Bundle) -> ImportReport` (`root` and one `{local, eid, new}` per statement). The facade trait `BundleFormat` adds `to_json`, `from_json` (`tiramemsu-bundle/1`) and `to_ntriples`. `View::dependents(eid)` is the read-only cascade preview of [[time-model#Cascade#Dependents]].
- A `View` is a pure value: creating or deriving one does no I/O. Rows from an as-of view report `t_ret` and `ret_kind` as absent, so each row shows what was believed then; `history()` gives real lifetimes.
- `SparqlResult` is `Solutions`, `Boolean`, `Graph` or `Update(TxReport)`, with `write_sparql_json` (SELECT, ASK) and `write_ntriples` (CONSTRUCT). A SPARQL update is one transaction on the writer and returns its `TxReport`. See [[query#Front Ends#SPARQL]].
- `sparql(q)` is `sparql_with(q, &SparqlOptions::default())`. `SparqlOptions { provenance: true }` makes each `SELECT` row carry the eids of the statements that produced it: `Solutions::provenance(row) -> Option<&[Eid]>`, a `"provenance"` member in SPARQL JSON, and `provenance: true` on the JSON bridge's `sparql`. `ASK`, `CONSTRUCT` and updates with it are `Unsupported`. See [[query#Front Ends#SPARQL#Query Provenance]].
- `values(s, key)` is how M0 exposes volatile state before a query language exists. See [[storage#Volatile Table]].
- `Patch::from_fields` builds a patch from named fields for bindings and rejects `s` and `p` with `InvalidPatch`.

`Tx` operations are specified in [[time-model#Operations]]. Values cross the API as `Value` (IRI, node, literal, statement, tx), which the ObjectId codec encodes. See [[data-model#ObjectId]].

## Errors

Every failure is a typed error, and a failed transaction leaves no trace: no tx row, no triples, no terms.

| Error | Raised when |
|---|---|
| `UniqueViolation { p, o, existing }` | Asserting a second live subject for a `sys:unique` predicate |
| `ValueTypeMismatch { p, expected, got }` | The object violates `sys:valueType` |
| `SubjectTypeMismatch { p, expected, got }` | The subject's kind is none of the predicate's `sys:subjectType` tags (a typed layer written on a node, say) |
| `CascadeLimitExceeded { root, limit }` | The cascade set is larger than `max_cascade` |
| `NotLive(eid)` | `supersede` or `confirm` on a retracted or unknown eid |
| `InvalidPatch` | A patch tries to change `s` or `p`, gives an empty interval (`v_from ≥ v_to`), or changes nothing |
| `SelfReference(eid)` | A statement would use its own eid as `s` or `o` |
| `ReservedNamespace(iri)` | User data writes a `sys:` predicate that is not a schema, vocab or prefix flag (or tx metadata on a tx), `sys:inGraph` (only `GRAPH` blocks, `WITH` and `Tx::add_to_graph` write it), any `tm:` predicate, a schema flag on a `sys:` subject, or supersedes an engine statement (`sys:confirmedBy`, `sys:supersedes`) |
| `InvalidGraphName { term }` | A graph name that is a literal, statement or transaction |
| `GraphNotFound { graph }` / `GraphExists { graph }` | `CLEAR GRAPH` or `DROP GRAPH` on a graph with no membership and no declaration; `CREATE GRAPH` on a declared graph (both unless `SILENT`) |
| `SchemaConflict { violating }` | A schema change is violated by existing live data |
| `Parse { dialect, span, msg }` / `Unsupported { feature }` | A query is outside the v1 subset. `dialect` is SPARQL, Cypher or Path (the `tm_path` expression text). `Unsupported` also rejects what format 1 reserves for later milestones: tag 15 `SEALED` and the `sys:sensitive` flag (M6) |
| `MissingCapability { capability }` | `Db::open` with the query engine on a host that lacks `functions` or `vtab`. See [[architecture#Executor]] |
| `InvalidQuery { msg }` | A structurally invalid IR or query plan (e.g. an unbound variable in a projection) that is not a parse error |
| `PathLimitExceeded { limit }` | A path search exceeds `OpenOptions.path_max_states` (default 1 000 000). Results are never silently truncated |
| `FormatVersion { found, supported }` | The file was written by a newer format |
| `ForeignFile` | The file is a SQLite database with user tables but no `meta` table |
| `InvalidTerm { position, reason }` | A value of the wrong kind in a position (e.g. a literal as predicate, a non-IRI volatile key), or an unknown dictionary id |
| `InvalidInterval` | `assert`/`create` with an empty valid interval (`v_from ≥ v_to`); `InvalidPatch` covers supersede |
| `NotUniquePredicate(p)` | `upsert` on a predicate without `sys:unique` |
| `Reentrant` | A write is started from inside a running transaction on the same `Db` |
| `DeleteConnectedNode { node, relationships }` | Cypher `DELETE n` while `n` still has live relationships at the end of the query (use `DETACH DELETE`); it lists their eids |
| `Eval { dialect, msg }` | A runtime expression error during query evaluation: a type error, integer division by zero, an unstorable property value or an invalid `@id` |
| `Sqlite(e)` / `Custom(msg)` | A host-neutral SQLite error carrying the result code (busy, I/O, corruption), or the caller aborting the transaction body |

The error enum is `#[non_exhaustive]`. Each OpenSpec change adds the variants it owns.

## Open Options

`OpenOptions` gathers the per-database tuning knobs. They are defined across the OpenSpec changes and listed here so bindings expose one consistent set.

| Option | Default | Owner |
|---|---|---|
| `readers` | 4 | `add-core-store` |
| `clock` | system clock (injectable for tests) | `add-core-store` |
| `busy_timeout` | 5 s | `add-core-store` |
| `term_cache_capacity` | 16 384 | `add-core-store` / `add-query-ir-and-sql-planner` |
| `optimize_every` | 1000 commits (also after a commit inserting that many statements) | `add-core-store` |
| `planner` | default routing (LFTJ off) | `add-query-ir-and-sql-planner` |
| `query_engine` | true; false opens the `tm-core` tier only, for hosts without `functions` or `vtab` | `add-query-ir-and-sql-planner` |
| `path_max_hops` | 15 | `add-path-engine` |
| `path_max_states` | 1 000 000 | `add-path-engine` |

## Bindings

Bindings wrap the facade crate one to one. Python and Node are implemented over a shared JSON bridge ([[bindings]]); the others are designed and ordered later. See [[overview#Open Inputs]].

| Binding | Crate | Notes |
|---|---|---|
| Python | `tiramemsu-python`, package `tiramemsu` (PyO3, maturin wheel) | Done. Transactions take a list of op dicts, or a context manager |
| Node | `tiramemsu-node`, package `@tiramemsu/node` (napi-rs) | Done. Sync API; queries return plain JS objects |
| WASM | `tiramemsu-wasm` | SQLite compiled to WASM with an OPFS VFS; single-threaded, reader = writer |
| MCP | `tiramemsu-mcp` (stdio JSON-RPC server) | Tools: `cypher`, `sparql`, `assert`, `retract`, `supersede`, `history`, `as_of`, `schema` |
| SQLite extension | later | Only the `tm_path` table function and time helpers; no write API |

## MCP Tools

The MCP server is the likely first consumer for LLM agents. Its tools are thin wrappers around `View` and `Tx`, with time as an explicit argument.

- `cypher(query, params?, as_of?, valid_at?)` and `sparql(query, as_of?, valid_at?)` return rows as JSON.
- `assert(s, p, o, valid_from?, valid_to?, meta?)`, `retract(eid, reason?)` and `supersede(eid, patch, reason?)` return the `TxReport`.
- `assert` and `sparql` take an optional `graph`: `assert` adds the statement to that graph (`Tx::add_to_graph`), and a search or `sparql` call filters to it (`FROM <graph>`). The MCP crate does not exist yet, so this is the contract it implements.
- `history(node_or_eid)` returns the event rows and the tx metadata that touch it.
- `schema()` lists predicates with their flags and usage counts.
