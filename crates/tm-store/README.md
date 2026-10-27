# tm-store

The typed storage contract of [Tiramemsu](https://github.com/Volland/tiramemsu) and the one transaction engine that runs over it.

- `ReadStore` and `WriteStore` are the storage traits: point reads, prefix scans with any bound positions, the log, and appends. There is no SQL in this crate, and no operation deletes or rewrites a statement, a term or a transaction.
- `mem::MemStore` is the in-memory reference implementation, used as the executable specification of the contract and in property tests.

Backends that implement the traits (redb in `tm-redb`) get the full transaction engine and its guarantees.
