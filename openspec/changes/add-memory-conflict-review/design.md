## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Build on existing view scans, author/source layers, schema checks, dependents, and dry runs. Report different objects for a subject/predicate with overlapping half-open valid intervals as potential disagreement; do not call multi-valued predicates schema violations automatically. Evidence remains attributed and never collapsed into an invented confidence score. Import previews use current dry-run semantics, including burned ids, and return proposed assertions, reuse, schema failures, and dependency changes. Applying later revalidates against the then-current database; a preview is not a reserved transaction. Bundles are interchange, not replica-log synchronization.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **Conflict inspection** is `View::conflicts(&ConflictQuery { subject, predicate, limit, confidence, source })` over `tm_core::conflict::inspect`: one self-join finds subject/predicate pairs with distinct objects and overlapping half-open valid intervals (the assert overlap test), then each pair is swept over its interval bounds into `overlaps`, the maximal windows where two or more objects hold. Statements outside every window are left out; same-object statements are grouped as support under one `ConflictValue`.
- **Evidence** per statement: eid, valid interval, `t_add` and instant, the largest numeric confidence layer or `None`, confirming transactions, distinct `sys:author` / `sys:source` of the asserting and confirming transactions, and the statement's source layer (`v:source` by default). The confidence and author reads are shared with text recall (`EvidenceReader`). No combined score exists.
- **Schema neutrality:** multi-valued predicates are reported like any other; `declared_many` says when `sys:cardinality sys:many` is declared. `sys:` predicates are skipped unless named. The history view is `Unsupported` because it mixes statements never believed together; now, as-of and valid-time views work, and inside `Db::with` the speculative state is seen.
- **Import preview** is `Db::preview_bundle(&Bundle) -> BundlePreview { import, report, failure, burned: IdUsage, scope: PreviewScope { basis, t, instant } }`: `Tx::import_bundle` inside `TxOptions { dry_run: true }`. Schema and write-semantics errors (`UniqueViolation`, `ValueTypeMismatch`, `SubjectTypeMismatch`, `ReservedNamespace`, `InvalidInterval`, `InvalidGraphName`, `SelfReference`) are reported as `failure`; malformed bundles (`InvalidTerm`, checked before the writer is taken), cycles (`Unsupported`), budgets and host errors are raised. `Tx::id_usage()` was added to disclose the burned ids.
- **Application** is the existing explicit write `tx.import_bundle` (`importBundle`, MCP `import_bundle`), which revalidates against the current state; no new apply API and no reservation were added. No format migration was needed.
- **Surfaces:** JSON bridge `conflicts` (read, view and budget) and `previewBundle` (`bundle`, `budget`); Node and Python wrappers with typed results; MCP read tools `conflicts` and `preview_bundle` (the latter takes the writer briefly and advances only burned id counters, so it is offered in read-only mode).
