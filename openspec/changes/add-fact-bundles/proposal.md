## Why

Two questions come up whenever an agent works with layered memory, and today neither has a good answer.

1. **"What would retracting this take with it?"** Retract cascades recursively over subject and object positions (`retraction-cascade`), so one retraction can remove a belief, its provenance, its confidence and every statement built on those. The only preview is `TxOptions.dry_run`, which runs the retraction on the single writer: it takes the write lock, allocates and burns ids, and cannot look at the past. An agent that wants to show "this fact supports 14 other statements" before acting, or an auditor who asks "what depended on this fact last Tuesday", needs a plain read on any view.
2. **"Hand this fact, with its evidence, to another agent."** Physical isolation per agent means one SQLite file per agent (`lat.md/data-model#Layers`). Eids, node ids and transaction ids are local to a file, so a fact cannot be copied between files by id, and copying only its triple loses the layers (source, confidence, the belief it supports, its graph membership) that make it worth copying. SPARQL `CONSTRUCT` exports triples, but it loses statement identity and valid time, and nothing imports it back as layers.

Both questions are the same walk: the statements that stand on a root statement. This change specifies that walk as a read, and builds a portable **fact bundle** on it.

## What Changes

**Part A: statement dependents (read-only impact analysis).**
- `read::dependents(exec, spec, root) -> Vec<Eid>` in `tm-core` and `View::dependents(eid)` in the facade: breadth-first from `root` over the statements visible in the view whose `s` or `o` is a visited eid, with a visited set so reference cycles terminate. The order is the cascade's: root first, then breadth-first, each expansion ordered by eid. An eid that is not visible in the view gives an empty list.
- Time selection comes from `scan_predicates` only, so the read works under now (what a retraction would take now), as-of (what depended on it then), history (everything that ever depended on it) and valid-at.
- Under the now view the result equals, as a set, the retracted set of a dry-run retraction of the same statement, and the ends of the path `(^sys:subject|^sys:object)*` from it.
- The read is unbounded and never truncated (design Decision 3).
- JSON bridge read op `dependents` (`eid`, `view`).

**Part B: fact bundles (move a belief with its evidence between databases).**
- `View::bundle(root) -> Bundle`: the statements that depend on `root` in the view plus the downward closure of the statements they reference, so an imported layer never dangles. Statements that cannot be moved are excluded, each for a stated reason: statements that reference a transaction (transaction ids are local to a file), engine bookkeeping other than graph membership (`sys:supersedes`, `sys:confirmedBy`), and anything that references an excluded or invisible statement, together with its dependents.
- `Bundle` (new `tm-core` module `bundle`): ordered statements `{ local, s, p, o, valid }` where a position is a value, a bundle-local statement id or a bundle-local anonymous node label, plus the local id of the root. Every statement comes after the statements it references when the references are acyclic.
- `Tx::import_bundle(&Bundle) -> ImportReport`: asserts every statement idempotently in the caller's transaction, mints fresh nodes for anonymous labels, adds memberships through `add_to_graph`, applies schema checks, and returns the mapping from local id to eid with a `new` flag. Importing the same bundle twice changes nothing the second time (for bundles without anonymous nodes); importing onto a database that already holds the root fact attaches the layers to the existing eid. A bundle with a reference cycle fails with `Unsupported` before anything is written.
- Serialization in the facade: `BundleFormat::to_json` / `from_json` with the versioned format `tiramemsu-bundle/1`, and `to_ntriples` for RDF 1.2 N-Triples interchange (reifier blank nodes, `rdf:reifies` triple terms, annotation triples), reusing the `tm-sparql` N-Triples writer.
- JSON bridge: read op `bundle` (`eid`, `view`) returning the bundle JSON, and transaction op `importBundle` returning the mapping.

## Capabilities

### New Capabilities
- `statement-dependents`: the dependents read on any view, its order, cycles, time selection, equivalence with the cascade, and the bridge op.
- `fact-bundles`: bundle membership and exclusions, the `Bundle` value, anonymous nodes, import semantics (idempotence, existing root, memberships, schema, cycles, atomicity), JSON and N-Triples forms, and the bridge ops.

### Modified Capabilities
- `retraction-cascade`: the dry-run preview requirement gains a read-only preview on any view, equal to the dry run under the now view.
- `json-bridge`: the call surface gains the reads `dependents` and `bundle`, and the transaction op list gains `importBundle`.

## Impact

- **`tm-core`:** `read::dependents` and `read::bundle` (reads, `scan_predicates` only), the `bundle` module (`Bundle`, `BundleStatement`, `BTerm`, `ImportReport`), `Tx::import_bundle` in a new `engine/bundle.rs`. No table, index, trigger or format change. `tm-core` keeps its dependency list (no `serde_json`).
- **Facade (`tiramemsu`):** `View::dependents`, `View::bundle`, the `BundleFormat` extension trait (JSON and N-Triples), re-exports. Adds `serde_json` as a dependency (already in the tree through `tm-cypher`).
- **JSON bridge (`bindings/json`):** two read ops and one transaction op. The Node and Python wrappers pass unknown ops through `call`, so they need no change in this change; typed wrapper methods are a follow-up.
- **Not affected:** the query languages, the path engine, the storage format, existing views and the cascade itself.
- **Docs:** `lat.md/time-model.md` (Cascade), `lat.md/data-model.md` (a Fact Bundles section), `lat.md/api.md`, `lat.md/tests.md`, when implemented.
