## Context

See `proposal.md` (Why) for the motivation. This section covers the state and constraints that shape the design.

- **Schema flags** are ordinary statements `(p flag o)` with the predicate IRI as subject (`lat.md/data-model#Predicate Schema`). `Tx::schema(p)` reads the live flags into a `PredicateSchema`, cached per transaction and cleared whenever a flag statement is inserted or any statement is retracted.
- **The write pipeline** (`Tx::write`, `lat.md/time-model#Operations#Assert`) runs, in order: positions, reserved namespace, interval, then either flag validation plus schema-change validation (for a flag statement) or the `sys:valueType` check (for any other statement), then idempotency, uniqueness, cardinality-one replacement and insert. A failed operation fails the whole transaction.
- **Flag statements are implicitly cardinality one:** a new value of a flag retracts the previous one with kind `cardinality` (`predicate-schema`, "Schema flag values are validated").
- **Subjects** are `IRI`, `NODE`, `BNODE`, `STMT` or `TX` ids (`lat.md/data-model#Statements`); `check_positions` already rejects any other subject.
- **Retracting a flag** only ever loosens the schema today, so the spec says it never conflicts.

## Goals / Non-Goals

**Goals:**
- Declare that a predicate annotates statements only (or only nodes, or transactions), enforced on every user write path.
- Reuse the flag machinery: versioned statements, validation, `SchemaConflict`, cache invalidation, `:Predicate`.
- Keep the check cheap: one tag comparison per write on a predicate that has the flag, nothing on one that has not.

**Non-Goals:**
- Constraining the subject's class (`rdf:type`) or the annotated statement's predicate ("`v:confidence` annotates only `v:worksAt` statements"). That is a shape language, not a tag check.
- Constraining engine writes (`sys:confirmedBy`, `sys:inGraph`). Their predicates are `sys:` IRIs, which cannot carry flags.
- Query-time use of the flag (planner hints, Cypher rendering). It only constrains writes.

## Decisions

### Decision 1: A tag IRI, restricted to subject kinds

The object is a tag IRI (`vocab::tag_iri`), the same vocabulary `sys:valueType` uses for tags. Only the five subject-capable tags are accepted. A literal tag (`sys:INT`) or a datatype IRI can never match a subject, so accepting it would declare a predicate that can never be written; it fails with `ValueTypeMismatch { p: sys:subjectType, expected: sys:IRI, got }`, as a bad `sys:valueType` value does.

### Decision 2: Several values mean any-of

`sys:subjectType` is the one multi-valued flag. A predicate that annotates both statements and transactions (a `v:note`) needs two values, and one tag IRI cannot say that. The alternatives, a union IRI (`sys:STMT_OR_TX`) or a list object, need new vocabulary or new object kinds. So the implicit cardinality one of flag statements is not applied to `sys:subjectType`: asserting a second value adds it, and re-asserting a live value is idempotent. `PredicateSchema::subject_types` holds the live values in eid order; an empty list means no constraint.

### Decision 3: Checked beside the value type

For a statement that is not a flag, the write pipeline checks the object against `sys:valueType` and then the subject against `sys:subjectType`, before the idempotency lookup, uniqueness and cardinality-one replacement. So a rejected write retracts nothing and an idempotent re-assert of a violating triple is rejected too, exactly like the value type. The error is `SubjectTypeMismatch { p, expected, got }`, where `expected` lists the allowed tag IRIs and `got` is the subject's tag.

Supersede keeps the subject, and the root was valid when it was written (schema changes are checked against live data), so supersede does not repeat the check.

### Decision 4: Schema changes are checked against the resulting set

The allowed set after a change is what matters:

- **Assert or create of a value** `T` on predicate `x`: the set becomes the live values ∪ {T}. Live statements of `x` whose subject tag is outside it conflict. Only the first value can conflict; later ones widen the set.
- **Supersede of a value** (`T` → `T'`): the set becomes the live values without the superseded statement ∪ {T'}.
- **Retraction of a value**: the set becomes the live values without it. If the set is then empty the constraint is lifted and nothing conflicts; otherwise the narrowed set is checked.

Each conflict fails with `SchemaConflict { violating }` listing the eids in ascending order, and the check sees earlier writes of the same transaction. The retraction check runs where every retraction root starts (`retract_root`), so explicit retraction, `retract_matching`, SPARQL `DELETE` and Cypher all go through it. A cascade never reaches a flag statement, because its subject is an IRI and its object a tag IRI, never an eid.

This changes the rule "retracting any flag SHALL never conflict": retracting one of several `sys:subjectType` values is the single exception.

### Decision 5: Same cache and reserved-namespace handling as the other flags

`sys:subjectType` joins `SCHEMA_FLAGS` (so inserting it clears the schema cache, and Cypher's `:Predicate` and the node scan treat it as a flag) and `SYS_ALLOWED` (so users may assert it). A flag on a `sys:` predicate stays `ReservedNamespace`.

## Risks / Trade-offs

- **A multi-valued flag is a special case.** Code that assumed one value per flag (the cardinality-one replacement of flag statements) needs the exception, and a reader of `(p sys:subjectType ?t)` gets several rows. The alternative encodings are worse (Decision 2).
- **Retraction now looks for a flag statement.** Each retraction root costs one dictionary probe for `sys:subjectType` (an index seek; a hit is cached, a miss is not) and, once the IRI exists, one primary-key lookup of the root. Cascaded members are not checked, because a cascade never reaches a flag statement.
- **The error lists IRIs as ids.** `expected` holds tag IRI ids, rendered like `ValueTypeMismatch`'s `expected`; the message also names the rejected tag.

## Migration Plan

Additive. No format change and no data migration. A store without `sys:subjectType` statements behaves as before. Before this change `sys:subjectType` was a reserved `sys:` predicate that users could not assert, so no existing data uses it.

## Open Questions

- Should Cypher `CREATE (n {confidence: 0.8})` on a node, where `v:confidence` is `sys:STMT`, get a friendlier message that suggests a relationship variable? The core error names the predicate and both kinds, which is enough for now.
