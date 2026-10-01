## 1. Core

- [x] 1.1 `engine/graph.rs::graph_id`: accept `Tag::Stmt`. For a statement graph, require it to be live (`NotLive(graph)` otherwise). Literals and `TX` stay `InvalidGraphName`.
- [x] 1.2 `engine/supersede.rs`: replay a `sys:inGraph` row when its object (the graph) is in the cascade set, with `s` and `o` mapped through σ. Keep dropping the others.
- [x] 1.3 Core tests: add to, read, remove from and clear a statement graph; the cascade on retracting the graph statement; `NotLive` for a retracted graph; supersede replays the contents and still drops a member's own memberships; `asOf` before and after.

## 2. SPARQL

- [x] 2.1 `dataset.rs::graph_name` and `update/run.rs::known_graph`: accept `Value::Stmt`.
- [x] 2.2 Flip the "statement is not a graph name" assertions (`lower_patterns.rs`, `dataset.rs` unit test, `sparql_graphs.rs`, `sparql_graph_update.rs`) to the transaction IRI, which stays invalid.
- [x] 2.3 Facade tests: `INSERT DATA { GRAPH <urn:tiramemsu:stmt:n> { … } }`, `GRAPH ?e { … }` with `?e` bound by `~ ?e`, `DELETE DATA` of the edge keeps the members, and `CLEAR GRAPH <stmt>`.

## 3. Documentation

- [x] 3.1 `lat.md/data-model.md#Named Graphs`: graph names, the liveness rule, the supersede rule, and the edge-as-container example. Decision row D36 in `lat.md/overview.md`.
- [x] 3.2 `lat.md/tests.md`: spec sections for the new tests, referenced from code.
- [x] 3.3 `article/tiramemsu-metagraph.md`: mark the lift as done.
- [x] 3.4 Site article `site/articles/metagraphs.html`, a metagraph recipe in `lat.md/recipes.md` tested by `metagraph_containers_nesting_and_fold`, and README and home page entries.
