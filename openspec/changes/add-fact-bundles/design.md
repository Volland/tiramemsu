## Context

See `proposal.md` (Why) for the motivation. This section covers the state and constraints that shape the design.

- **Layers are statements about statements.** A statement's `s` or `o` may be another statement's eid (`lat.md/data-model#Layers`), nesting has no depth limit, and longer reference cycles (e7 about e8 about e7) are allowed. Only the self-reference is rejected.
- **The cascade is the only walk over layers today.** `Tx::cascade_set` (`crates/tm-core/src/engine/cascade.rs`) walks breadth-first from a root over live statements whose `s` or `o` is a visited eid, one `UNION` query per visited eid, each expansion ordered by eid, bounded by `max_cascade`. It runs on the writer only.
- **Dry run is speculation.** `TxOptions.dry_run` runs the whole transaction on the writer inside a savepoint and burns the ids it allocated (`lat.md/time-model#Speculative Transactions`). It holds the single write lock and always starts from now.
- **One function writes time predicates.** `scan_predicates` (`crates/tm-core/src/view.rs`) turns a `ViewSpec` into SQL for one `triple` alias; every read in `tm-core` and every scan in `tm-exec` uses it (`lat.md/query#Views and Scans`).
- **The path engine already walks layers.** `View::path` with the virtual hops `^sys:subject` and `^sys:object` reaches the same statements (`layer-hops`), but it lives in `tm-exec`, needs the query engine and the `vtab` capability, and `tm-core` must run on a host with no capability (`lat.md/tests#Storage Invariants#Core Runs On A Minimal Host`).
- **Ids are local to one file.** Eids, `NODE` and `BNODE` counters and transaction numbers are allocated per database. Skolem IRIs (`urn:tiramemsu:node:<n>`, `stmt:<n>`, `tx:<t>`) parse back to the inline id of the *same* file, so exporting them would silently alias unrelated things in another file.
- **Assert is idempotent** on a live `(s, p, o)` with an overlapping valid interval (`lat.md/time-model#Operations#Assert`), and memberships are written only by `Tx::add_to_graph` and SPARQL graph updates (`named-graphs`).
- **`tm-core` depends on `thiserror` and `lru` only.** The facade has the query crates; `tm-cypher` already depends on `serde_json`, and `tm-sparql` has an RDF 1.2 N-Triples writer (`results::nt::write`) with triple terms.

## Goals / Non-Goals

**Goals:**
- A read, on any view and any connection, that lists what a retraction would take with it now, or what depended on a statement at any past time.
- A portable, self-contained, versioned unit that moves a fact with its layers and its evidence from one database file to another, and imports idempotently.
- No new table, column, index, trigger or format change. Time semantics come from the existing view function.

**Non-Goals:**
- Replication or sync of whole databases (the event log is the base for that later).
- Moving history: a bundle is a snapshot of one view; transaction times and lifetimes are not carried, and imported statements get the importing transaction's time.
- Importing arbitrary RDF 1.2 N-Triples as a bundle. N-Triples is an export for interchange; the JSON form is the one that round-trips.
- Merging anonymous nodes across imports (blank-node isomorphism).
- Typed Node and Python wrapper methods (the ops work through the generic `call`).

## Decisions

### Decision 1: Dependents is the cascade walk, as a read on a view

`read::dependents(exec, spec, root)` returns `root` followed by every statement reached breadth-first over "statements visible in `spec` whose `s` or `o` is a visited eid". Each expansion is one `SELECT … WHERE a.s = ?e AND <view> UNION SELECT … WHERE a.o = ?e AND <view> ORDER BY eid`, exactly the shape of `cascade_set` with `t_ret IS NULL` replaced by `scan_predicates(spec, "a")`. A visited set makes cycles terminate. `root` itself must be visible in the view, otherwise the result is empty: there is nothing to retract, and "what depended on it" has no meaning at a time it was not believed.

Consequences: under the now view the set equals the cascade set, so a dry-run retraction and `dependents` agree by construction, and the property test checks it (Decision 7). Under as-of the walk is the structure as it was; under history it is every statement that ever stood on the root, including ones retracted before the root; under valid-at it keeps statements valid at the instant, so a layer valid in another period does not appear.

Alternatives considered:
- *Run the path engine* (`(^sys:subject|^sys:object)*` in `REACH` mode). Same set, but `tm-core` cannot depend on `tm-exec`, the path engine needs the `vtab` capability, and `REACH` orders by hop count then id, not by the cascade's per-expansion eid order. The path form is kept as the third leg of the equivalence test.
- *A recursive CTE.* Rejected for the same reason the path engine avoids them (`lat.md/query#Physical Planning#Path Engine`): no visited set across branches, and the order would differ.
- *Batch the frontier through `rarray`.* Needs the `functions` capability, which `tm-core` must not assume. Per-eid probes on `live_spo`/`live_osp` (or `hist_*`) match the cascade's cost.

### Decision 2: Time predicates only through `scan_predicates`

Both halves of the `UNION` and the root visibility check get their time predicates from `scan_predicates`, so the now shape keeps `t_ret IS NULL` verbatim and the partial `live_*` indexes apply, and `AsOf(Instant)` resolves inside the read's own statement. No new time SQL is written.

### Decision 3: The read is unbounded; it never truncates

`dependents` has no limit parameter. A truncated impact list is wrong in the way that matters most (it under-reports what a retraction removes), and a failing read is unhelpful to a caller who asked precisely because the structure may be large. The cost is bounded by the statements that actually stand on the root, and the read holds no write lock and no transaction beyond its read snapshot. A caller who wants the cascade guard compares `len()` with its own `max_cascade`; the write path keeps enforcing `CascadeLimitExceeded`. This matches the path engine's rule that results are never silently truncated (`lat.md/query#Physical Planning#Path Engine`).

Alternative considered: `dependents(eid, limit)` failing with `CascadeLimitExceeded` past `limit`. Rejected: it names a write error on a read, and every caller would pass `usize::MAX` except the ones who want a count, who are better served by the full list.

### Decision 4: A bundle is the dependents plus their downward closure

`View::bundle(root)` collects:
1. `dependents(root)` in the view (the layers, beliefs and memberships that stand on the root), then
2. the downward closure: every statement referenced in `s` or `o` by a collected statement, transitively, that is visible in the view.

Step 2 makes the bundle self-contained: a belief `(b supportedBy e1)` that is the root brings `e1`, and a layer on a layer brings its base, so no imported statement points at an eid that does not exist in the target. Step 2 does *not* add the dependents of the referenced statements. Taking them would pull in the whole connected component (every other belief that cites `e1`, their layers, and so on), which is a database copy, not a fact.

### Decision 5: Exclusions, each with a reason

A candidate statement is excluded when:
- **Its `s` or `o` is a transaction** (`TX` tag), e.g. `(e1 sys:confirmedBy tx5)` or `(tx5 sys:author v:agent7)`. Transaction numbers are local to one file; tx5 in the target is an unrelated transaction, and a transaction cannot be created by a write.
- **Its predicate is engine bookkeeping that a user cannot write**, i.e. a `sys:` or `tm:` predicate that the reserved-namespace check refuses for a user write, except `sys:inGraph`. This removes `sys:supersedes` (a link between two eids of the source's history, meaningless without that history) and `sys:confirmedBy`. The rule is "a bundle holds only what import can write again": `sys:inGraph` is kept because import writes it through `add_to_graph`, and user-writable `sys:` flags are kept because `assert` accepts them.
- **It references an excluded statement or a statement that is not visible in the view** (a dangling eid, or one retracted before the view's time). It would dangle in the target.

Exclusion propagates to dependents: a statement whose `s` or `o` is an excluded statement is excluded, to a fixpoint, and the downward closure is then recomputed from the surviving statements so nothing is carried only because an excluded statement referenced it. If the root itself is excluded, `bundle` fails with `Unsupported` naming the reason; if the root is not visible in the view it fails with `NotLive(root)`.

### Decision 6: Terms travel as values; anonymous nodes as bundle-local labels

`IRI` and literal ids are decoded to `Value`s, which the target encodes in its own dictionary. `STMT` ids become bundle-local statement ids (`BTerm::Stmt(local)`). `NODE` and `BNODE` ids become bundle-local anonymous labels (`BTerm::Node(label)`), numbered from 0 in order of first appearance, and import mints one fresh `NODE` per label. Exporting their skolem IRIs instead would collide: `urn:tiramemsu:node:5` names node 5 of the source, and importing it would attach the fact to whatever node 5 is in the target.

Consequences, documented:
- Two anonymous nodes stay distinct, and one node used by two statements of a bundle stays one node in the target.
- Import of anonymous nodes is not idempotent: importing the same bundle twice mints new nodes the second time, so statements that mention them are new again. This is RDF blank-node semantics (loading the same Turtle file twice duplicates its blank-node triples). Bundles without anonymous nodes import idempotently.
- The `NODE`/`BNODE` distinction is not carried; both are anonymous nodes to both query languages, and import mints `NODE` ids.
- An import that names a skolem IRI (`urn:tiramemsu:stmt:…`, `node:`, `bnode:`, `tx:`) as a value is rejected with `InvalidTerm`, since it would silently alias a local id.

### Decision 7: Local ids and order

Local ids are the positions `0..n` of the statements in the bundle. The order is a topological order of the reference graph (a statement after the statements it references), with ties broken by source eid, and statements in a cycle appended in eid order after everything else. Anonymous labels are numbered in the same pass. The same view of the same data therefore always yields the same bundle, so JSON output is stable and can be diffed.

### Decision 8: Import is the assert pipeline in the caller's transaction

`Tx::import_bundle(&Bundle)` validates the whole bundle first (local ids, references, the predicate is an IRI, values are not skolem ids, the order is acyclic), then walks it in order: each statement's positions are mapped (value → encode, local statement → its mapped eid, anonymous label → minted node), a `sys:inGraph` statement goes through `add_to_graph(member, graph, valid)`, and every other statement through `assert(s, p, o, valid)`. So:
- Importing twice asserts `Existing` everywhere the second time and changes nothing.
- Importing onto a database that already holds the root fact (same `(s, p, o)`, overlapping valid time) returns that eid, and the layers are asserted onto it. The existing statement's valid time is not widened (assert semantics).
- Valid time is carried; transaction time is the importing transaction's.
- Schema applies: `sys:valueType`, `sys:unique` and `sys:cardinality one` in the *target* database. A violation fails the operation, and the caller's transaction rolls back as a whole. A `sys:one` predicate in the target retracts the target's overlapping value, as any assert does.
- Reserved-namespace checks apply as for any user write.
- The report maps every local id to `(eid, new)`, in bundle order, plus the root's eid.

### Decision 9: Reference cycles are rejected on import

A cycle (e7 about e8 about e7) cannot be asserted idempotently: asserting e7 needs e8's eid and vice versa. Supersede's replay handles cycles by pre-allocating every eid (the substitution map σ) and inserting rows directly, but that is `create` semantics: it always makes new statements, and deciding whether an equal cycle already exists in the target is a subgraph-isomorphism question. So import fails with `Unsupported { feature: "bundle with a reference cycle" }` before any write. Export keeps cycles (the bundle is still an exact description), so the limit is on import only.

### Decision 10: Serialization lives in the facade

`tm-core` keeps its minimal dependency list, so the JSON and N-Triples forms are an extension trait `BundleFormat` in the facade (the pattern of `TxCypher`), implemented for `tm_core::Bundle`:
- `to_json() -> serde_json::Value` and `from_json(&serde_json::Value) -> Result<Bundle>` with the format below. `from_json` rejects another format string, a missing field or a malformed term with `InvalidTerm`, and checks references as import does.
- `to_ntriples() -> String`: each statement `i` is written as its triple, plus `_:s<i> rdf:reifies <<( s p o )>>`, plus `_:s<i> tm:validFrom/tm:validTo "…"^^xsd:dateTime` when bounded. A position that is a statement is written as that statement's reifier `_:s<j>`, so a layer `(e1 confidence 0.8)` is the annotation triple `_:s0 v:confidence 0.8`; an anonymous node is `_:n<k>`. Terms are rendered with the `tm-sparql` term renderer and written by its N-Triples writer, so the output is RDF 1.2 N-Triples that `oxttl` parses.

```json
{
  "format": "tiramemsu-bundle/1",
  "root": 2,
  "statements": [
    { "id": 0, "s": {"iri": "urn:tiramemsu:v:alice"}, "p": "urn:tiramemsu:v:worksAt",
      "o": {"iri": "urn:tiramemsu:v:acme"}, "validFrom": 1704067200000 },
    { "id": 1, "s": {"ref": 0}, "p": "urn:tiramemsu:v:confidence",
      "o": {"lex": "0.8", "datatype": "http://www.w3.org/2001/XMLSchema#double"} },
    { "id": 2, "s": {"blank": 0}, "p": "urn:tiramemsu:v:supportedBy", "o": {"ref": 0} }
  ]
}
```

A term is `{"iri"}`, `{"ref": local}` (a statement of the bundle), `{"blank": label}` (an anonymous node), or a literal `{"lex", "datatype"}` / `{"lex", "lang"}`. Literals always carry their datatype so the round trip is exact. `validFrom` / `validTo` are epoch milliseconds, absent when unbounded. The version is in the string; a reader refuses versions it does not know.

### Decision 11: JSON bridge

- Read `dependents` (`eid`, `view`) returns a list of eids.
- Read `bundle` (`eid`, `view`) returns the bundle JSON of Decision 10.
- Transaction op `importBundle` (`bundle`) returns `{"root": eid, "statements": [{"id", "eid", "new"}]}`, and may carry `as`, which names the imported root for later ops.

## Risks / Trade-offs

- **Large bundles and dependents.** Both are unbounded reads (Decision 3). A hub statement with a million layers gives a million-entry result; that is the true answer, and the read holds no write lock. Mitigation if needed later: a streaming variant.
- **Semantic duplicates on import.** Assert matches by `(s, p, o)` and overlapping valid time. A layer imported onto an existing fact that already has a different confidence yields two confidences. That is the data model (assert is not merge), and cardinality-one predicates in the target resolve it where declared.
- **Schema differences between files.** The same bundle may import into one file and fail in another (`UniqueViolation`, `ValueTypeMismatch`). Failure is atomic, and the error names the predicate.
- **Anonymous nodes are not idempotent** (Decision 6). Documented, tested.
- **Cycles cannot be imported** (Decision 9). They are rare (they need a write that names a future eid, or a supersede of such a structure).

## Migration Plan

Additive. No format change, no data migration. The new reads and the import op are opt-in; existing behaviour is unchanged.

## Open Questions

- **Carrying transaction metadata.** A bundle could carry `sys:author`/`sys:source` of the transactions that added its statements as plain provenance layers (for example `v:importedFrom`). Left out: it changes the statements' meaning, and the caller can add such layers on the imported root.
- **Importing N-Triples.** An RDF 1.2 reader that rebuilds a bundle from reifiers is possible (`rdf:reifies` gives the triple, annotations give the layers) and is a natural follow-up.
