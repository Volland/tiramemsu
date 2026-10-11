# Lean model of the store contract

`Tiramemsu/Store/` is a pure model of the storage contract the Rust engine relies on: the
format-1 rows (`Types`), the per-index-family key order and scan predicates (`Order`), the
abstract read/write/snapshot store classes (`Interface`) and `ModelStore`, the store every
theorem is stated over (`Model`). `Tiramemsu/Sqlite/Conn.lean` connects the same interface
to SQLite through leansqlite and is not built by default.

```sh
cd lean
lake build            # the pure model (TiramemsuStore)
```

The design is in `lat.md/architecture.md#Store Abstraction`.
