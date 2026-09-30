## Why

Layers are how Tiramemsu attaches confidence, provenance and beliefs to facts: a layer statement has a statement eid as subject, `(e1 v:confidence 0.8)` (`lat.md/data-model#Layers`). Nothing says that `v:confidence` is a layer predicate. A writer can just as well assert `(v:alice v:confidence 0.8)`, and the store accepts it. A query that reads confidence through `?e v:confidence ?c` then mixes statement annotations with node properties, and a Cypher `r.confidence` and `n.confidence` mean different things with no schema to say so.

The predicate schema already constrains the object of a predicate with `sys:valueType` (`lat.md/data-model#Predicate Schema`). The subject is the missing half. A per-statement store can say "this predicate annotates statements only", which is what makes a layer a typed layer: `(v:confidence sys:subjectType sys:STMT)`.

## What Changes

- A new schema flag `sys:subjectType` whose object is a subject-capable tag IRI: `sys:IRI`, `sys:NODE`, `sys:BNODE`, `sys:STMT` or `sys:TX`. Any other object fails with `ValueTypeMismatch`, like other bad flag values.
- Several live `sys:subjectType` statements on one predicate mean any-of: `(v:note sys:subjectType sys:STMT)` and `(v:note sys:subjectType sys:TX)` let `v:note` annotate statements and transactions. Unlike the other flags, a new value does not replace the previous one.
- Assert and create (and therefore SPARQL `INSERT`, Cypher `CREATE`, `SET` and `MERGE`) check the subject's tag against the flag before insert, next to the `sys:valueType` check. A violation fails with a new error `SubjectTypeMismatch { p, expected, got }` that lists the allowed tag IRIs and the rejected subject's tag.
- A schema change that live data violates is rejected with `SchemaConflict { violating }`: asserting the first `sys:subjectType` on a predicate that already has statements with other subject kinds, and also retracting or superseding one value of several, which narrows the allowed set.
- `sys:subjectType` is assertable by users like the other schema flags, it is versioned and queryable, the schema cache follows it, and Cypher's `:Predicate` label includes predicates that carry only this flag.
- The JSON bridge maps the new error to the code `"SubjectTypeMismatch"`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `predicate-schema`: flags gain `sys:subjectType` (a multi-valued flag), its value validation, the subject-type check, the schema-change rule including narrowing on retraction, and its place in the order of checks.
- `json-bridge`: the error-code list gains `SubjectTypeMismatch`.

## Impact

- **`tm-core`:** `vocab` (`SYS_SUBJECT_TYPE`, `SCHEMA_FLAGS`, `SYS_ALLOWED`), `engine/schema.rs` (`Flag::SubjectType`, `PredicateSchema::subject_types`, the check and the conflict rules), the write and supersede pipelines, the retraction of a flag statement, and the `Error::SubjectTypeMismatch` variant.
- **`bindings/json`:** one line in the error-code mapping.
- **`tm-cypher`:** no code change; `:Predicate` and the node scan read `SCHEMA_FLAGS`.
- **`tm-sparql`:** no code change; updates go through `Tx::assert` and `Tx::retract`.
- **Not affected:** the storage format, the ObjectId encoding, queries, the node and Python binding sources (they carry the bridge's error code through unchanged, so the new code reaches them without a change).
- **Docs:** `lat.md/data-model.md` (Predicate Schema table, Layers), `lat.md/time-model.md` (the assert flow), `lat.md/tests.md`.
