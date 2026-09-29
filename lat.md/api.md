# API

The `tiramemsu` facade crate exposes three handles: `Db` opens the file and runs transactions, `View` is an immutable time selection that runs queries, and `Tx` holds the write operations.

## Rust Surface

The signatures below are the contract that bindings wrap. Names are fixed; details such as generic bounds may change during implementation.

```rust
pub struct Db { /* writer + reader pool */ }

impl Db {
    pub fn open(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Db>;
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
}
```

`Tx` operations are specified in [[time-model#Operations]]. Values cross the API as `Value` (IRI, node, literal, statement, tx), which the ObjectId codec encodes. See [[data-model#ObjectId]].

## Errors

Every failure is a typed error, and a failed transaction leaves no trace: no tx row, no triples, no terms.

| Error | Raised when |
|---|---|
| `UniqueViolation { p, o, existing }` | Asserting a second live subject for a `sys:unique` predicate |
| `ValueTypeMismatch { p, expected, got }` | The object violates `sys:valueType` |
| `CascadeLimitExceeded { root, limit }` | The cascade set is larger than `max_cascade` |
| `NotLive(eid)` | `supersede` or `confirm` on a retracted eid |
| `InvalidPatch` | A patch tries to change `s` or `p`, or gives an empty interval (`v_from ≥ v_to`) |
| `SelfReference(eid)` | A statement would use its own eid as `s` or `o` |
| `ReservedNamespace(iri)` | User data asserts a `sys:` predicate that is not a schema, vocab or prefix flag |
| `SchemaConflict { violating }` | A schema change is violated by existing live data |
| `Parse { dialect, span, msg }` / `Unsupported { feature }` | A query is outside the v1 subset |
| `FormatVersion { found, supported }` | The file was written by a newer format |

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
