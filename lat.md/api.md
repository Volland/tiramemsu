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
}

impl View {
    pub fn valid_at(self, epoch_ms: i64) -> View;
    pub fn sparql(&self, q: &str) -> Result<QueryResult>;
    pub fn cypher(&self, q: &str, params: &Params) -> Result<QueryResult>;
    pub fn path(&self, start: ObjectId, path: &str, mode: PathMode, max_hops: u32) -> Result<Vec<PathRow>>;
    pub fn triples(&self, s: Option<ObjectId>, p: Option<ObjectId>, o: Option<ObjectId>) -> Result<Vec<Triple>>;
    pub fn values(&self, s: ObjectId, key: ObjectId) -> Result<Vec<ObjectId>>; // statements, else volatile (Now only)
    pub fn encode(&self, v: &Value) -> Result<Option<ObjectId>>;              // lookup only, never inserts
    pub fn decode(&self, id: ObjectId) -> Result<Value>;
    pub fn events_since(&self, t: u64) -> Result<Vec<Event>>;
}
```

`Tx` is the core write handle, re-exported by the facade. Besides the operations of [[time-model#Operations]] it offers `assert_with` (with `OnExisting::Confirm`), `new_bnode`, `clear_volatile`, `encode`, `lookup`, `decode`, `schema`, `t` and `instant`. Positions take any `IntoObject`: an `ObjectId`, `Eid`, `TxId` or `Value`.

- A `View` is a pure value: creating or deriving one does no I/O. Rows from an as-of view report `t_ret` and `ret_kind` as absent, so each row shows what was believed then; `history()` gives real lifetimes.
- `values(s, key)` is how M0 exposes volatile state before a query language exists. See [[storage#Volatile Table]].
- `Patch::from_fields` builds a patch from named fields for bindings and rejects `s` and `p` with `InvalidPatch`.

`Tx` operations are specified in [[time-model#Operations]]. Values cross the API as `Value` (IRI, node, literal, statement, tx), which the ObjectId codec encodes. See [[data-model#ObjectId]].

## Errors

Every failure is a typed error, and a failed transaction leaves no trace: no tx row, no triples, no terms.

| Error | Raised when |
|---|---|
| `UniqueViolation { p, o, existing }` | Asserting a second live subject for a `sys:unique` predicate |
| `ValueTypeMismatch { p, expected, got }` | The object violates `sys:valueType` |
| `CascadeLimitExceeded { root, limit }` | The cascade set is larger than `max_cascade` |
| `NotLive(eid)` | `supersede` or `confirm` on a retracted or unknown eid |
| `InvalidPatch` | A patch tries to change `s` or `p`, gives an empty interval (`v_from ≥ v_to`), or changes nothing |
| `SelfReference(eid)` | A statement would use its own eid as `s` or `o` |
| `ReservedNamespace(iri)` | User data writes a `sys:` predicate that is not a schema, vocab or prefix flag (or tx metadata on a tx), any `tm:` predicate, a schema flag on a `sys:` subject, or supersedes an engine statement (`sys:confirmedBy`, `sys:supersedes`) |
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
| `DeleteConnectedNode(node)` | Cypher `DELETE n` while `n` still has relationships (use `DETACH DELETE`) |
| `Eval { msg }` | A runtime expression error during query evaluation |
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
| `path_max_hops` | 15 | `add-path-engine` |
| `path_max_states` | 1 000 000 | `add-path-engine` |

## Bindings

Bindings wrap the facade crate one to one. Which binding ships first is an open input, so all of them are designed now and ordered later. See [[overview#Open Inputs]].

| Binding | Crate | Notes |
|---|---|---|
| Python | `tiramemsu-py` (PyO3, maturin wheel) | Transactions take a list of op dicts, or a context manager |
| Node | `tiramemsu-node` (napi-rs) | Sync API; queries return plain JS objects |
| WASM | `tiramemsu-wasm` | SQLite compiled to WASM with an OPFS VFS; single-threaded, reader = writer |
| MCP | `tiramemsu-mcp` (stdio JSON-RPC server) | Tools: `cypher`, `sparql`, `assert`, `retract`, `supersede`, `history`, `as_of`, `schema` |
| SQLite extension | later | Only the `tm_path` table function and time helpers; no write API |

## MCP Tools

The MCP server is the likely first consumer for LLM agents. Its tools are thin wrappers around `View` and `Tx`, with time as an explicit argument.

- `cypher(query, params?, as_of?, valid_at?)` and `sparql(query, as_of?, valid_at?)` return rows as JSON.
- `assert(s, p, o, valid_from?, valid_to?, meta?)`, `retract(eid, reason?)` and `supersede(eid, patch, reason?)` return the `TxReport`.
- `history(node_or_eid)` returns the event rows and the tx metadata that touch it.
- `schema()` lists predicates with their flags and usage counts.
