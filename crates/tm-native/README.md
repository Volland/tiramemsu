# tm-native

> **Experimental in 0.8.** The redb backend passes the same oracle and differential suites as SQLite, but its file format may still change, and saved answers are not supported on it yet. Measurements: `bench/backends/README.md` in the repository.

The native query runner of [Tiramemsu](https://github.com/Volland/tiramemsu): it runs the whole query IR over the `tm-store` storage traits, without SQL.

It reuses the planner and the result decoder of `tm-exec`, evaluates expressions with the same value rules and the same function library as the SQL route, and joins triple patterns on the store's permutations: index nested loops and hash joins for acyclic patterns, and a worst-case-optimal multiway join for cyclic ones.
