## Dependencies

- Option A (reserve origin bits) was chosen in design.md, so this change goes ahead.
- Independent of every open change. Fact bundle import (`fact-bundles`, already archived) applies the origin check of task 1.3.

## 1. Core

- [x] 1.1 Add `ObjectId::origin()` and `ObjectId::counter()` for `NODE`, `BNODE`, `STMT` and `TX` (`payload >> 48`, `payload & (2⁴⁸ − 1)`), with unit tests at the bounds.
- [x] 1.2 Bound the allocators in `engine/mod.rs` (`alloc_eid`, node, bnode, `t`) so that the largest number allocated is 2⁴⁸ − 1, and add `Error::IdSpaceExhausted { kind }`. Test by seeding `meta.next_stmt` (and `next_node`, `next_bnode`, `last_t`) in a fixture.
- [x] 1.3 Reject a non-zero origin with `Unsupported { feature: "origin <n> ..." }`: skolem IRIs (parsed in `codec.rs`, rejected where the value is encoded), `Value::Stmt`/`Node`/`BNode`/`Tx` in the term dictionary on the write and read paths, raw ids and eids in every write operation, and bundle import.
- [x] 1.4 Map `IdSpaceExhausted` in `bindings/json` error names.

## 2. Documentation

- [x] 2.1 `lat.md/data-model.md#ObjectId`: document the origin split and the bound; add a decision row (D35) to `lat.md/overview.md`.
- [x] 2.2 `lat.md/tests.md`: spec sections for the bound and the origin rejection, referenced from the tests.
