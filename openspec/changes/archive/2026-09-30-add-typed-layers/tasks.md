## Dependencies

- M0 (`add-core-store`, archived) owns the predicate schema and the write pipeline; this change extends them and needs no storage change.
- Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. The requirement text is in `specs/*/spec.md` of this change.

## 1. Vocabulary and errors

- [x] 1.1 Add `SYS_SUBJECT_TYPE` to `tm-core` `vocab`, to `SCHEMA_FLAGS` (so the schema cache and Cypher's `:Predicate` follow it) and to `SYS_ALLOWED` (so users may assert it). Extend the reserved-namespace unit test.
- [x] 1.2 Add `Error::SubjectTypeMismatch { p, expected: Vec<ObjectId>, got: Tag }` with a message that names the predicate, the allowed tag IRIs and the rejected tag. Unit-test the message.
- [x] 1.3 Map the variant to `"SubjectTypeMismatch"` in `bindings/json` and test the code end to end.

> Notes (group 1): the message is `subject type mismatch on {p}: expected one of [IRI:n, …], got IRI`, rendering ids like `ValueTypeMismatch` does. It is checked in the core test `subject_type_constrains_subjects` and in the SPARQL test, not in a unit test of `error.rs` (which has none). The bridge test is `subject_type_mismatch_has_its_own_code` in `bindings/json/tests/bridge.rs`.

## 2. Schema engine

- [x] 2.1 Add `Flag::SubjectType` and `PredicateSchema::subject_types` (live values in eid order), read by `Tx::schema`.
- [x] 2.2 Validate the flag value: a tag IRI of `IRI`, `NODE`, `BNODE`, `STMT` or `TX`, else `ValueTypeMismatch` for `sys:subjectType`.
- [x] 2.3 Exempt `sys:subjectType` from the implicit cardinality one of flag statements, in assert and in supersede, so values accumulate (any-of).
- [x] 2.4 Check the subject's tag in the write pipeline right after the value type, before idempotency, uniqueness and cardinality, and fail with `SubjectTypeMismatch`.
- [x] 2.5 Reject schema changes that live data violates with `SchemaConflict`, computing the resulting set for an assert, a supersede (without the superseded value) and a retraction (without the retracted value; an empty set lifts the constraint).
- [x] 2.6 Tests in `crates/tm-core/tests/predicate_schema.rs`: accepted and rejected subjects, any-of, value validation, no replacement of a previous value, in-transaction effect, conflicts on assert, supersede and retraction, lifting by retracting the last value, the order of checks, and cache behaviour within one transaction.

> Notes (group 2): `Flag::single_valued` drives the implicit cardinality one in `Tx::write` and `Tx::supersede`. `validate_schema_change` takes a new `replacing: Option<Eid>` (the superseded flag statement, `None` from `write`). The retraction check is `Tx::validate_flag_retraction`, called at the top of `retract_root` in `cascade.rs`, so `retract`, `retract_matching`, SPARQL `DELETE` and Cypher deletes all pass through it; it returns at once when `sys:subjectType` was never interned or the root is not a subject-type flag. The engine's own writes (`sys:confirmedBy`, memberships) go through the same check but can never have a flag, because their predicates are `sys:` IRIs. A `PredicateSchema` doctest shows a typed layer. The tests run on both hosts (`host_test!`).

## 3. Front ends

- [x] 3.1 SPARQL: a rejected `INSERT DATA` of a node-level triple and an accepted annotation (`{| v:confidence 0.8 |}`) on a statement.
- [x] 3.2 Cypher: a rejected `CREATE` of a node property and an accepted `SET r.confidence` on a relationship.

> Notes (group 3): no front-end code change. SPARQL: `typed_layer_rejects_a_node_subject` in `crates/tiramemsu/tests/sparql_update_tx.rs` also checks that the whole multi-operation request leaves no trace. Cypher: `typed_layer_rejects_a_node_subject` in `crates/tiramemsu/tests/cypher_write_set.rs` covers `CREATE` and `SET` on a node, then `SET r.confidence`.

## 4. Documentation

- [x] 4.1 Update `lat.md/data-model.md`: the Predicate Schema table row and bullets, and "typed layers" in Layers.
- [x] 4.2 Update `lat.md/time-model.md#Assert` (the order of schema checks and the assert flow).
- [x] 4.3 Add `lat.md/tests.md` sections for the new tests with `@lat` references, and run `lat check`.

> Notes (group 4): new `tests#Typed Layers` section with six leaves (core constraint, value validation, schema changes, SPARQL, Cypher, bridge). `lat check` passes.
