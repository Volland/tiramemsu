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
