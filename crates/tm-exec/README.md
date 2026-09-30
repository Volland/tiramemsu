# tm-exec

The tiramemsu query executor: turns a logical IR query into one parameterised SQL statement over the bitemporal `triple` table, runs it, and decodes the result.

`tm-exec` takes an [`tm_ir::IrQuery`], validates it, binds parameters, routes each region to generated SQL or to the native path operator, resolves time in one place, and returns typed rows. It reaches SQLite only through the [`tm_core::Executor`] trait, so it contains no database driver.

```text
  tm-sparql / tm-cypher / tiramemsu API
                  |   IrQuery (tm-ir)
                  v
   +--------------------------------------+
   |  tm-exec   <- you are here           |
   |  validate > bind > route > resolve   |
   |  > normalise > SQL > run > decode    |
   +------------------+-------------------+
                      |  Executor trait (tm-core)
                      v
        tm-rusqlite host  ->  one SQLite file
```

Most applications should use the [`tiramemsu`](https://docs.rs/tiramemsu) facade crate: `db.now().execute_ir(..)` and `explain_ir(..)` wrap this crate. Use `tm-exec` directly when you are embedding the engine in another host (implementing the `Executor` and host-registry traits), adding a native operator, or testing planner output.

## Usage

Run an IR query through the facade (which owns a [`QueryEngine`]), then ask for its plan. `tiramemsu` and `tm-rusqlite` supply the connection here; `tm-exec` does the work.

```rust
use tiramemsu::{Db, OpenOptions, TxOptions, Valid, Value};
use tm_exec::RegionKind;
use tm_ir::builder::IrBuilder;
use tm_ir::Params;

let dir = tempfile::tempdir().unwrap();
let db = Db::open(dir.path().join("x.db"), OpenOptions::default())?;
let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    Ok(())
})?;

let b = IrBuilder::sparql();
let q = b.query(b.triple("?a", "v:worksAt", "?c"));

// Run: rows of optional cells, decoded to typed values.
let r = db.now().execute_ir(&q, &Params::new())?;
assert_eq!(r.get(0, "c"), Some(&v("acme")));
assert!(r.stats.sql_executed);

// Explain: routing, SQL text, bound parameters and SQLite's query plan. Nothing is stepped.
let e = db.now().explain_ir(&q, &Params::new())?;
assert_eq!(e.regions[0].kind, RegionKind::Sql);
assert!(e.sql.as_deref().unwrap().contains("triple"));

// A constant that is in no statement makes the pattern empty: no SQL runs at all.
let q = b.query(b.triple("?a", "v:worksAt", "v:never-seen"));
let e = db.now().explain_ir(&q, &Params::new())?;
assert!(e.short_circuit && e.sql.is_none());
# Ok::<(), tiramemsu::Error>(())
```

## Tour

- [`QueryEngine`]: one database's engine. `prepare` (validate, bind, precheck), `execute`, `explain`, `install` (register helpers on a connection).
- [`Prepared`], [`ExecContext`]: a validated, bound query and the connection it runs on.
- [`QueryResult`], [`ResultValue`], [`ExecStats`]: decoded rows (`None` cell = missing), terms or lists, counters.
- [`Explain`], [`RegionInfo`], [`RegionKind`], [`RouteNote`]: what `explain` returns.
- [`plan`]: parameter binding, constant encoding, view resolution, normalisation, routing.
- [`sqlgen`]: the SQL generator. [`scan`]: the view to SQL predicate mapping.
- [`virtual_pred::VirtualPred`]: `sys:subject`, `tm:txAdded` and the other computed predicates.
- [`decode`]: [`TermCache`], [`CacheMode`] and ObjectId decoding.
- [`native`]: [`NativeOperator`], [`OperatorRegistry`], [`NativeKind`], [`PlannerOptions`], [`LftjConfig`].
- [`host`]: capability check and installation. [`path`]: [`PathEngine`], [`PathRequest`], [`PathRow`], the `tm_path` table function.
- [`udf`], [`udf_fn`]: the SQL scalar and aggregate helper functions registered on every connection.

## Design notes

- **Pipeline.** validate, bind parameters, route (paths go to the native `tm_path` operator), resolve views and encode constants on the executing connection, normalise (empty regions propagate), generate SQL, run, decode. Planning, the statement and decoding use one connection and so one snapshot.
- **Time in one place.** Only [`scan::view_predicates`] writes time predicates; it delegates to `tm_core::scan_predicates`. `AsOf(Instant)` becomes a transaction number by reading the `tx` table on the executing connection; an instant before the first transaction makes that pattern empty.
- **Constants and short-circuit.** Constants are encoded to ObjectIds at plan time by lookup only; planning never inserts a term. A constant that is not in the dictionary cannot match, so its pattern becomes empty and, if the whole query is empty, no SQL runs (`Explain::short_circuit`). All values are SQL parameters; the SQL text contains no data.
- **Join order needs statistics.** Constants are bound parameters, so SQLite orders joins well only with `ANALYZE` data (STAT4). `tm-exec` does not generate hints; the database layer (`tiramemsu`'s `Db`) keeps statistics current. If you drive this crate against your own connection, run `ANALYZE` on populated files or expect slow plans on skewed data. Stale statistics change speed, never results.
- **Set versus bag.** Under `SetOfTriples` duplicate `(s, p, o)` are removed only for predicates listed in the `pred_multi` table (predicates that have ever held two statements with the same triple). Other predicates skip the extra lookup. Under `BagOfEids` every statement matches.
- **Virtual predicates** are read from the statement row, not stored, and cost no joins. They are also how paths cross layers (`sys:subject`, `sys:object`, `sys:predicate`).
- **Native operators.** Regions that SQL handles badly run as table-valued functions behind [`NativeOperator`]. Only the path operator exists; LFTJ routing is a config placeholder and never produces a region today. Hosts must provide the `functions` and `vtab` capabilities; [`host::install`] fails with `MissingCapability` rather than degrade.
- **Errors** are the shared `tm_core::Error`: `InvalidQuery`, `Unsupported`, `Sqlite`, `MissingCapability`, `PathLimitExceeded`.
- **Explain** returns each region's kind, the SQL, bound parameters and `EXPLAIN QUERY PLAN` with the real parameters.

## Status

Version 0.1. The public API, especially [`plan`], [`sqlgen`] and [`native`], is exposed for testing and hosts and may change between 0.x releases. Only SQLite (through `tm-rusqlite`) is exercised. Minimum supported Rust version: 1.88.

## Links

- Repository: <https://github.com/Volland/tiramemsu>
- Planning and paths: <https://github.com/Volland/tiramemsu/blob/main/lat.md/query.md>
- Storage and indexes: <https://github.com/Volland/tiramemsu/blob/main/lat.md/storage.md>
- IR crate: <https://docs.rs/tm-ir>

## License

MIT OR Apache-2.0
