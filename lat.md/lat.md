This directory defines the high-level concepts, business logic, and architecture of this project using markdown. It is managed by [lat.md](https://www.npmjs.com/package/lat.md) — a tool that anchors source code to these definitions. Install the `lat` command with `npm i -g lat.md` and run `lat --help`.

- [[overview]] — What Tiramemsu is, its goals and non-goals, and the record of every design decision
- [[architecture]] — Layers, crates, connections and concurrency, and deployment
- [[data-model]] — Statements with eids, layers, the ObjectId encoding, nodes, vocabulary mapping, and the predicate schema
- [[time-model]] — Transaction time, valid time, operations, cascade, the event log, never-forget, and speculation
- [[storage]] — The SQLite schema, indexes, query shapes, event view, invariant triggers, and the volatile table
- [[query]] — The IR, views and scans, physical planning, the path engine, the SPARQL and Cypher front ends, and temporal syntax
- [[api]] — The Rust facade, errors, bindings, and MCP tools
- [[bindings]] — The JSON bridge shared by the Node.js and Python packages: operations, views, terms, errors, and how each wrapper is tested
- [[prior-art]] — Systems studied, and what was taken from or avoided in each
- [[recipes]] — Tested queries that combine statement ids, layers, transaction metadata, paths and time scopes
- [[tests]] — Test specifications for the core invariants
- [[roadmap]] — Milestones mapped to OpenSpec changes, and benchmarks
- [[paper]] — The Layered Bitemporal Graphs preprint: source, verification artifact, arXiv package and its page on the site
- [[project-review]] — Measured architecture review, resource-safety findings, and proposed feature priorities

- [[paper]] — Research manuscript artifacts and the isolated arXiv source-package workflow.
- [[code-ontology]] — shared vocabulary of intent types and typed edges between intent and code
- [[backends]] — Decisions D37–D44: the typed storage trait, the redb backend, the native runner, durability modes and backend detection
- [[reasoning]] — Decisions N1–N18: N3 reasoning with eyeron over views, pointwise valid time, stored rule sets and materialized, grounded conclusions
- [[datalog]] — Decisions DL1–DL24 (proposed): a fast Datalog crate with DatalogMTL over valid time, magic-set goals, constraints and reconciled materialization
- [[constraints]] — Project-level constraints that bind several decisions, with their sources
- [[changes]] — Changes that moved intent and code together, cited by `changed_by` edges
- [[derived-beliefs]] — Decisions D45–D52: derived beliefs with `sys:derivedFrom` support links, the support cascade, supersede, bundles and derived upsert
