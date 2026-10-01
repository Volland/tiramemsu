## Why

Tiramemsu has half of a metagraph: an edge is a vertex, because every statement has an eid that other statements can point at. The other half is containers. A named graph is already a metavertex (a node with statements tagged into it), but only a node can be a graph name, so an edge cannot hold a subgraph. An enrollment edge `(p7 enrolledIn trial3)` cannot own its investigator and site statements. The workaround is a context node `(e v:context v:enroll1)` with the contents in `GRAPH v:enroll1`. The context node can drift from the edge, and `supersede` does not carry it along.

`article/tiramemsu-metagraph.md` names this as the only engine change the core metagraph story needs.

## What Changes

- A **statement eid is a valid graph name.** `Tx::add_to_graph`, `remove_from_graph`, `clear_graph`, `create_graph`, `drop_graph`, SPARQL `GRAPH <urn:tiramemsu:stmt:n>`, `GRAPH ?e` with `?e` bound to a statement, `FROM`, `FROM NAMED`, `USING`, `WITH`, `CREATE`, `CLEAR` and `DROP` all accept a `STMT` id. Literals and transactions stay `InvalidGraphName`.
- **Adding to a statement-named graph needs the graph statement to be live.** Otherwise the call fails with `NotLive(graph)`. Retracting the graph statement already cascades to its memberships, because a membership's object is the graph. So "the container is gone, the members stay" needs no new code.
- **Supersede carries the contents of a corrected edge.** A membership whose graph is in the cascade set is replayed onto the new eid. A membership of a superseded *member* statement in some other graph is still dropped, as before.
- No format change, no new column, no new error variant.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `named-graphs`: graph names include statement eids; the liveness rule for statement-named graphs; supersede replays the memberships of a statement-named graph.

## Impact

- **`tm-core`:** `engine/graph.rs` (graph name validation and the liveness check) and `engine/supersede.rs` (membership replay). About 30 lines.
- **`tm-sparql`:** `dataset.rs::graph_name` and `update/run.rs::known_graph` accept `Value::Stmt`.
- **Tests:** the existing "statement is not a graph name" assertions flip. New tests cover edge-as-container, cascade, supersede and time travel.
- **Docs:** `lat.md/data-model.md#Named Graphs`, a decision row D36, and test specs.
- **Not in scope:** a deep graph selector (`within*` sugar), metavertex-rooted bundles, and Cypher syntax for containment. The dual view already reaches the memberships. N-Quads export does not exist yet. When it lands, a statement graph renders as `urn:tiramemsu:stmt:<n>`.
