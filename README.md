<p align="center"><img src="site/assets/logo.svg" width="180" alt="Tiramemsu: a brain taking a bite out of a tiramisu whose layers are a graph"></p>

<h1 align="center">Tiramemsu</h1>

<p align="center"><em>Your agent’s brain loves Tiramemsu.</em></p>

<p align="center"><strong>Layered, never-forget memory for agents.</strong><br>
An embedded graph database on SQLite. Every fact has an id, so facts can carry layers of provenance and belief, and every change is kept with when it was made and when it was true.</p>

<p align="center"><a href="site/articles/layered-graphs.html">Layered graphs, explained</a> · <a href="site/articles/tiramemsu-vs-oxilite.html">Tiramemsu vs oxilite</a> · <a href="lat.md/">Design (lat.md)</a> · <a href="openspec/specs/">Specs (OpenSpec)</a></p>

> **Status: new.** Designed and implemented in September 2026 as one Rust library. It is not published to crates.io, has no server or bindings yet, and runs on rusqlite only. What is measured and what is not is listed under [Status](#status).

## Why

A knowledge graph stores facts. An agent also needs to say *how sure am I*, *where did I read it*, *what do I conclude from it*, and later *I was wrong*. Those are statements about statements. In tiramemsu a fact is a row `(eid, s, p, o)` with its own id, and an id can be the subject or object of another statement. That gives you **layered graphs** with no second data structure:

<p align="center"><img src="site/assets/layers.svg" width="640" alt="Three layers: the fact alice worksAt acme with id e1, a confidence and a source about e1, and a belief supported by e1"></p>

## Features

- **Layers.** Provenance, confidence and beliefs stack to any depth. Retracting a fact retracts its layers, and correcting it replays them.
- **Bitemporal.** Transaction time (when the database believed it) and valid time (when it was true). `now`, `as_of(t)`, `history` and `valid_at(d)` views, per query or per pattern.
- **Never forget.** SQLite triggers reject `DELETE` and any second change to a row, inside the file. Forgetting means retracting. Erasure for legal reasons is planned as crypto-shredding (destroy a key, keep the rows).
- **Memory verbs.** Idempotent `assert`, `create` for parallel edges, `supersede` (correct a fact and replay its layers), `confirm` (another source agrees), cardinality-one and unique predicates, and `with` / `dry_run` for changes that leave no trace.
- **SPARQL and Cypher, one store.** SPARQL 1.1 with RDF 1.2 annotations, and openCypher, share one logical IR and one semantics table. A relationship is also a `:Statement` node, so Cypher can reach layers.
- **Paths.** Reachability, trails and shortest paths through a native automaton search, also as a SQL table function (`tm_path`). Paths can cross layers.
- **Named graphs as tags.** A graph is a node and membership is one more layer statement. `GRAPH`, `FROM`, `FROM NAMED` and `WITH` work with no new column or table.
- **Embedded.** One SQLite file (WAL, STRICT). The core reaches SQLite through a small synchronous executor trait.

### What agents are saying

Early feedback from the people it is for. We made these up, but each one is about something the code really does.

> “I used to say *‘I recall you said Acme’* with total confidence. Now I say *0.8, from chat-2026-09-29*. My humans find this either reassuring or unsettling.” — a chatbot, on layers

> “I was wrong about where Alice works. `supersede` fixed the fact and carried my confidence over to it. I have never felt so forgiven.” — an assistant, on supersede

> “My last memory store let me `DELETE` things. I do not trust myself with that kind of power. Here the triggers say no, in the file itself.” — a cautious agent, on never forgetting

> “A user asked what I believed last Tuesday. I ran `as_of` and answered without a single hallucination. I asked for a raise in tokens.” — a support agent, on bitemporal views

> “I wanted to try a wild idea without committing to it. A `with` block let me be reckless and leave no trace.” — a planner agent, on speculation

> “My orchestrator speaks SPARQL and my intern speaks Cypher. They now share one store and, for the first time, one opinion.” — a multi-agent swarm, on two query languages

## Architecture

```mermaid
flowchart TD
    A["SPARQL 1.1 + 1.2<br/>tm-sparql"] --> IR
    B["openCypher<br/>tm-cypher"] --> IR
    C["Rust API<br/>Db · View · Tx"] --> IR
    IR["Logical IR (tm-ir)<br/>patterns · joins · paths · view per pattern · semantic flags"]
    IR --> P["Planner + SQL codegen<br/>tm-exec"]
    IR --> PE["Path engine<br/>automaton search · tm_path"]
    P --> V["View-aware scan<br/>now · as_of · history × valid_at"]
    PE --> V
    V --> X["Executor trait"]
    X --> S[("One SQLite file<br/>triple · term · tx · triggers")]
```

| Crate | Role |
|---|---|
| `tm-core` | ObjectId codec, term dictionary, format-1 schema and triggers, the transaction engine, views, the `Executor` trait. No SQLite dependency. |
| `tm-rusqlite` | The rusqlite host: bundled SQLite, functions, virtual tables. |
| `tm-ir` | Logical algebra, per-pattern views, semantic flags. |
| `tm-exec` | Planner, SQL codegen, path engine, result decoding. |
| `tm-sparql` | SPARQL front end. |
| `tm-cypher` | openCypher front end. |
| `tiramemsu` | The facade: `Db`, `View`, `Tx`. The only crate an application needs. |

Each layer depends only on the one below it, and time is resolved in exactly one place. The full design is in [`lat.md/`](lat.md/), and every capability has a spec under [`openspec/specs/`](openspec/specs/).

## Quick start

From [`crates/tiramemsu/examples/quickstart.rs`](crates/tiramemsu/examples/quickstart.rs). Run it with `cargo run -p tiramemsu --example quickstart`.

```rust
let db = Db::open(dir.join("memory.db"), OpenOptions::default())?;

// 1. A fact is a statement with its own id, so it can carry layers.
db.transact(TxOptions::default(), |tx| {
    let eid = match tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)? {
        Asserted::New(e) | Asserted::Existing(e) => e,
    };
    tx.assert(Value::Stmt(eid), v("confidence"), &conf, Valid::ALWAYS)?;
    tx.assert(Value::Stmt(eid), v("source"), Value::str("chat-2026-09-29"), Valid::ALWAYS)?;
    Ok(())
})?;

// 2. Correct it. The layers are replayed on the new fact; nothing is deleted.
let corrected = db.transact(TxOptions::default(), |tx| {
    tx.supersede(fact, Patch { o: Some(v("globex")), ..Patch::default() })?;
    Ok(())
})?;

// 3. What is believed now, and what was believed before the correction?
let q = "SELECT ?who ?org WHERE { ?who v:worksAt ?org }";
db.as_of(TimeRef::Tx(corrected.t.0 - 1)).sparql(q)?;   // alice → acme
db.now().sparql(q)?;                                   // alice → globex

// 4. The same store in Cypher. The confidence layer is a relationship property.
db.now().cypher("MATCH (p)-[r:worksAt]->(o) RETURN p, o, r.confidence", &CypherParams::default())?;
```

Layer queries in SPARQL use RDF 1.2 annotation syntax, and Cypher sees the id as a relationship and as a `:Statement` node:

```sparql
SELECT ?c ?s WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ; v:source ?s |} }
```

```cypher
MATCH (a)-[r:worksAt]->(c), (b:Belief)-[:supportedBy]->(r) RETURN a, c, r.confidence, b
```

## Build and test

Rust 1.91.1 is pinned in `rust-toolchain.toml`. If your shell finds another `rustc` first (for example Homebrew's), put rustup's first on `PATH`, because `open-cypher` needs 1.88 or newer.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace
cargo test --workspace                 # 927 tests; PROPTEST_CASES=64 to speed up property tests
cargo clippy --workspace --all-targets -- -D warnings
lat check                              # design graph and code refs stay in sync
```

## Status

| | |
|---|---|
| Size | About 58 000 lines of Rust in 7 crates, 927 tests, 30 capability specs |
| SPARQL | 634 of 781 in-scope W3C tests pass (66 more are skipped: named-graph data and unsupported formats). Every failing one is listed with a reason in `crates/tm-sparql/tests/w3c/expected-deviations.toml`, and an unexpected result fails the build |
| Cypher | 2 615 of 3 880 openCypher TCK scenarios (67 %). Temporal types, `CALL`, and a few dual-view cases are deferred and listed in `crates/tm-cypher/tests/tck/allowlist.txt` |
| Speed | Raw SQLite lookups on the schema take about 4 µs at 11 million statements; about 150 bytes per statement with all indexes. See [`bench/`](bench/) |

**Known limits.** As-of lookups slow down as one key collects many updates. A membership per statement roughly doubles the file. SPARQL decimals come back as doubles. Path patterns inside `GRAPH` are unsupported. There is no MCP server, binding or network server yet. The comparison with oxilite's change-log approach to history is not benchmarked head to head.

## How it differs from oxilite

[oxilite](https://github.com/Volland/oxilite) is a shipped, Oxigraph-compatible RDF database on SQLite by the same author. It stores quads and uses reifiers for annotations, keeps history in a change log, and runs on D1, WebAssembly and more. Tiramemsu stores statements with ids, keeps lifetime in the row and is bitemporal, but it is new, runs on rusqlite only and has none of oxilite's reasoning, validation or bindings. [The article](site/articles/tiramemsu-vs-oxilite.html) has the details and a benchmark.

## Repository

| Path | Contents |
|---|---|
| `crates/` | The seven crates |
| `lat.md/` | The design as a cross-linked knowledge graph: architecture, data model, time model, storage, query, tests |
| `openspec/specs/` | Requirements with scenarios, one folder per capability; `openspec/changes/archive/` has the changes that built them |
| `bench/` | SQLite versus DuckDB, and ids versus reifiers, benchmarks with results |
| `docs/design-options.md` | The option analysis behind the design decisions |
| `site/` | The static website (GitHub Pages) and its articles |

## Website

`site/` is plain HTML and one stylesheet, with no build step, no scripts and no external fonts. `.github/workflows/pages.yml` publishes it to GitHub Pages on pushes to `main` that touch `site/`. Enable it once under *Settings → Pages → Source: GitHub Actions*.

## License

MIT OR Apache-2.0.
