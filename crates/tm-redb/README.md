# tm-redb

> **Experimental in 0.8.** The redb backend passes the same oracle and differential suites as SQLite, but its file format may still change, and saved answers are not supported on it yet. Measurements: `bench/backends/README.md` in the repository.

The [redb](https://github.com/cberner/redb) storage backend of [Tiramemsu](https://github.com/Volland/tiramemsu).

It implements the `tm-store` storage traits on one redb file: statements in all six live `(s, p, o)` orders and three history orders, exact per-predicate statistics kept in every write transaction, the term dictionary, the transaction log, and a text index that tokenises and ranks like SQLite FTS5. The transaction engine is `tm-store`'s, so a redb database has the same semantics, ids and history as a SQLite one.
