## Context

A membership is `(m, e, sys:inGraph, g)` ([[data-model#Named Graphs]]). The graph name `g` is validated to be an `IRI`, `NODE` or `BNODE` in `Tx::graph_id` and in the SPARQL front end (`graph_name`, `known_graph`). Nothing else in the read or write path assumes the graph is a node. The `GRAPH` lowering is a join on `m.o = g`, `graph_members` and `graphs` are plain SQL over `triple`, and the cascade retracts every live statement whose subject or object is a retracted eid.

## Goals / Non-Goals

**Goals:**
- An edge can hold a subgraph: `GRAPH <stmt> { … }` writes and reads memberships whose graph is a statement.
- The container's lifetime follows the edge, and its contents survive a correction of the edge.

**Non-Goals:**
- A deep graph selector, metavertex bundles, or Cypher containment syntax.
- Changing what happens to a *member's* memberships under supersede (they are still dropped).

## Decisions

### D36: a statement is a graph name

Allow `Tag::Stmt` in graph-name validation. Transactions (`TX`) and literals stay invalid: a transaction is not a fact and cannot be retracted, and a literal is not addressable.

### Liveness: the graph statement must be live when a membership is added

`add_to_graph(e, g)` with `g` a statement fails with `NotLive(g)` when `g` is retracted or unknown. Combined with the existing cascade (a membership has `o = g`), every live membership of a statement-named graph has a live graph statement. Under `asOf`, the membership and the graph statement were retracted in the same transaction, so transaction-time views agree without a new join.

Valid time is not clamped. A membership keeps its own valid time, like any membership. A `validAt` view can therefore show contents of an edge whose own valid time has ended. That is consistent with how layers behave today: an annotation does not inherit the valid time of what it annotates.

*Alternative rejected:* a read-time join that hides memberships whose statement graph is not visible. It costs a join on every `GRAPH` read, and it only changes valid-time views, where layers already behave the other way.

### Supersede replays the memberships whose graph is in the cascade set

The cascade set `C` of a root already contains every membership whose graph is a member of `C`, because their object is in `C`. Today supersede skips every `sys:inGraph` row during replay. The new rule is to skip a membership only when its graph is **not** in `C`. A membership `(m, x, inGraph, g)` with `g ∈ C` is replayed as `(σ(m), σ(x) or x, inGraph, σ(g))`. The member statements themselves are not in `C` (unless they reference the root), so they keep their eids.

- An edge correction keeps its contents: the investigator is still the investigator after a typo fix in the trial id.
- A member statement superseded on its own still drops its memberships, as the named-graphs spec says.

## Risks / Trade-offs

- [A statement can be a member of its own graph, `(e sys:inGraph e)`] → Harmless. Retracting `e` cascades the membership. It is allowed rather than special-cased.
- [`graphs()` now lists statement ids] → Callers that decode graph ids already handle `Value::Stmt` (it is a valid `Value`). The SPARQL result renders it as `urn:tiramemsu:stmt:<n>`.
- [`max_cascade` now counts contents] → A large container makes supersede of its edge larger. It was already counted, because the memberships were in the cascade set; they were only not replayed.
