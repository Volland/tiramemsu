# tm-core

The engine under tiramemsu: ObjectId codec, term dictionary, storage format, transaction engine and temporal views, with no SQLite binding of its own.

Tiramemsu is a layered, never-forget memory for agents: every fact is a statement `(eid, s, p, o)` with its own id, so statements can be about statements, and nothing is ever deleted. `tm-core` is the lowest layer of that stack.

```text
 tiramemsu      facade: Db, View, Tx        <- most applications stop here
    |
 tm-sparql / tm-cypher --> tm-ir --> tm-exec   (query languages, planner, paths)
    |
 tm-core        ids, terms, schema, Store/Tx, views, Executor trait   <- this crate
    |  (Executor / Host traits)
 tm-rusqlite    the rusqlite host: bundled SQLite
```

Most applications should use the [`tiramemsu`](https://crates.io/crates/tiramemsu) facade crate. Use `tm-core` directly when you want only the storage engine (transactions, views, the event log) without SPARQL, Cypher or paths, or when you are writing a new SQLite host by implementing [`Host`] and [`Executor`].

## Usage

`tm-core` runs on any [`Host`]. This example uses `tm-rusqlite` (a dev-dependency of this crate; add it to your own `Cargo.toml` to run it).

```rust
use tm_core::{vocab::v, Store, StoreOptions, TxOptions, Value, Valid, ViewSpec};
use tm_rusqlite::RusqliteHost;

# let dir = tempfile::tempdir()?;
# let path = dir.path().join("memory.db");
let mut store = Store::open(&RusqliteHost::new(), &path, StoreOptions::default())?;

// One transaction: a fact, then a statement about that fact (a layer).
let report = store.transact(TxOptions::default(), |tx| {
    let fact = tx.assert(Value::iri(v("alice")), Value::iri(v("worksAt")), Value::iri(v("acme")), Valid::ALWAYS)?.eid();
    tx.assert(fact, Value::iri(v("confidence")), Value::Int(80), Valid::ALWAYS)?;
    Ok(())
})?;
assert_eq!(report.asserted.len(), 2);
let fact = report.asserted[0];

// Forgetting is retracting: the row stays, with a retraction time.
store.transact(TxOptions::default(), |tx| {
    tx.retract(fact)?;
    Ok(())
})?;
let now = store.read(|e| tm_core::read::triples(e, &ViewSpec::now(), None, None, None))?;
let history = store.read(|e| tm_core::read::triples(e, &ViewSpec::history(), None, None, None))?;
assert_eq!((now.len(), history.len()), (0, 2)); // the layer was retracted with its fact
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Tour

- [`Store`], [`StoreOptions`], [`Tx`]: the single-writer engine. `Store::transact` runs one transaction with one gap-free [`TxId`]; `Store::speculate` and `TxOptions::dry_run` run changes that leave no trace.
- Write verbs on [`Tx`]: `assert` (idempotent), `create` (parallel edges), `retract` (cascades to layers), `supersede`, `confirm`, `upsert`, graph memberships, volatile state.
- [`ObjectId`], [`Eid`], [`TxId`], [`Tag`]: the id types.
- [`Value`]: a decoded term (IRI, node, integer, string, date, and so on); [`mapping`] and [`vocab`] hold IRI conventions.
- [`ViewSpec`], [`TimeRef`], [`TxSel`], [`ValidSel`], [`view::scan_predicates`]: the now, as-of and history views crossed with a valid-time filter. `scan_predicates` is the one place time predicates are written.
- [`read`]: lookups over a view (`triples`, `values`, `events_since`, graph listings).
- [`TxReport`], [`Triple`], [`Valid`], [`Patch`], [`Asserted`], [`RetKind`], [`Event`]: operation arguments and results.
- [`Executor`], [`Host`], [`Capabilities`], [`HostRegistry`], [`SqlValue`]: the SQLite boundary.
- [`Clock`], [`SystemClock`], [`ManualClock`]: transaction instants; the manual clock makes tests deterministic.
- [`Error`]: every failure (see below).

## Design notes

- **ObjectId.** A 64-bit integer, `(payload << 4) | tag`. The low 4 bits are the [`Tag`]: IRI, node, blank node, statement, transaction, integer, boolean, date-time, date, short string (up to 7 bytes, inline) and the dictionary-backed string, language string, typed, double and decimal kinds. Tag 15 is reserved for sealed (erasable) values and currently rejected with `Error::Unsupported`. Small values are inline and never touch the term table.
- **Statements.** A row has an eid, `(s, p, o)`, a transaction lifetime `t_add..t_ret` and a half-open valid interval `[v_from, v_to)`. The eid can be the subject or object of another statement; that is the whole layering mechanism. Content never changes after insert.
- **Never forget.** SQLite triggers in the file reject `DELETE` and any change to a row other than setting `t_ret` and `ret_kind` once. Migrations may add but never rewrite triples. Legal erasure is planned as crypto-shredding and is not implemented.
- **Concurrency.** One writer per store; wrap [`Store`] in a mutex to share it. Readers are separate connections through [`Host::open_reader`].
- **Executor and capabilities.** The engine needs prepared statements, interactive transactions, savepoints and stable read snapshots. Optional [`Capabilities`] (`reader_pool`, `functions`, `vtab`, `stat4`, `fts5`) are declared by the host and never probed. `tm-core` needs none of them; only the opt-in text index ([`text`]) uses `fts5`, and a host without it never sees FTS5 SQL.
- **Format versioning.** `meta.format_version` is 2 (format 2 adds the bookkeeping of the derived text index). A file written by a newer format fails to open with `Error::FormatVersion`; older files are migrated forward in one transaction.
- **Text recall.** [`text::search`] recalls the statements whose string object (inline or dictionary, plain or language-tagged) matches a query, through the view's time predicates, with a lexical score and evidence (confidence, confirmations, authors, `t_add`), ranked by a documented policy with the eid as the final tie-break. The FTS5 index `term_fts` is derived: [`Store::rebuild_text_index`] rebuilds it without touching history. A SQLite file with foreign tables fails with `Error::ForeignFile`.
- **Errors.** [`Error`] is `#[non_exhaustive]`. Any error rolls the transaction back and leaves no trace; ids seen in a failed body may be reissued. Some variants (`Parse`, `PathLimitExceeded`, `MissingCapability`) are produced by higher crates and live here so all layers share one type.

## Status

Version 0.1: the API may change between minor versions. Format 1 is what the tests pin, but it has not seen production use. MSRV is Rust 1.88. Only the rusqlite host exists. The full design, in [`lat.md`](https://github.com/Volland/tiramemsu/tree/main/lat.md), covers the [data model](https://github.com/Volland/tiramemsu/blob/main/lat.md/data-model.md), [storage](https://github.com/Volland/tiramemsu/blob/main/lat.md/storage.md) and [time model](https://github.com/Volland/tiramemsu/blob/main/lat.md/time-model.md).

Project home: <https://github.com/Volland/tiramemsu>. License: MIT OR Apache-2.0.
