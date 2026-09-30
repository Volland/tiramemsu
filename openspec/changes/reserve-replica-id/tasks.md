## Dependencies

- **Blocked on the owner's decision** between Option A (reserve origin bits) and Option B (merge by translation) in design.md. Nothing here is started. If Option B is chosen, this change is withdrawn and no code changes.
- Independent of every open change. `add-fact-bundles` import must apply the origin check of task 1.3 once this lands.

## 1. Core

- [ ] 1.1 Add `ObjectId::origin()` and `ObjectId::counter()` for `NODE`, `BNODE`, `STMT` and `TX` (`payload >> 48`, `payload & (2⁴⁸ − 1)`), with unit tests at the bounds.
- [ ] 1.2 Bound the allocators in `engine/mod.rs` (`alloc_eid`, node, bnode, `t`) at 2⁴⁸ − 1 and add `Error::IdSpaceExhausted { kind }`. Test by seeding `meta.next_stmt` to 2⁴⁸ − 1 in a fixture.
- [ ] 1.3 Reject a non-zero origin in the skolem IRI parser (`mapping.rs`), in `Value::Stmt`/`Value::Node` encoding on the write path, and in bundle import, with `Unsupported { feature: "origin <n>" }`.
- [ ] 1.4 Map `IdSpaceExhausted` in `bindings/json` error names.

## 2. Documentation

- [ ] 2.1 `lat.md/data-model.md#ObjectId`: document the origin split and the bound; add a decision row (the next free D number) to `lat.md/overview.md`.
- [ ] 2.2 `lat.md/tests.md`: spec sections for the bound and the origin rejection, referenced from the tests.
