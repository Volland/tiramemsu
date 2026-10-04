# Architecture

Tiramemsu is a stack of layers: two query front ends over one logical IR, a planner that routes to SQL or native operators, a temporal view layer, executor hosts, and one SQLite file.

It follows MillenniumDB's logical design (edge ids, tagged ObjectIds, permutation indexes, automaton paths) on top of SQLite's B-trees instead of custom storage. See [[prior-art#MillenniumDB]].

## Layers

Each layer depends only on the layer below it. Time is resolved in exactly one place, the view-aware scan in [[query#Views and Scans]].

The agent memory services of 0.3 sit beside the Rust API in the facade and reuse the lower layers instead of adding new storage paths: budgets meter every layer, saved answers re-run queries, review and preview run dry transactions, and text recall is one more native operator over the same views. The derived tables can always be rebuilt from the graph ([[storage#Format Versioning]]).

```plantuml
@startuml layers
skinparam componentStyle rectangle
skinparam shadowing false

package "Front ends" {
  [SPARQL front end\n(spargebra → IR)] as SPARQL
  [Cypher front end\n(openCypher subset → IR)] as CYPHER
  [Rust API\n(Db / View / Tx)] as API
}

package "Agent memory services (facade)" {
  [Budget meter\n(QueryBudget, per thread)] as BUDGET
  [Saved answers\n(check / refresh)] as SAVED
  [Conflict review +\nbundle preview] as REVIEW
  [Bulk import\n(write lease)] as IMPORT
}

package "Logical layer" {
  [IR algebra\nBGP · Join · LeftJoin · Filter · Union\nAggregate · Path · Text\n+ semantic flags + per-pattern View] as IR
}

package "Physical layer (tm-exec)" {
  [Planner / router] as PLAN
  [SQL codegen] as SQLGEN
  [Path operator\n(tm_path)] as PATH
  [LFTJ operator\n(tm_lftj, opt-in)] as LFTJ
  [Text recall\n(tm_text)] as TEXT
}

package "Core (tm-core)" {
  [Temporal view\nnow · asOf · validAt · history] as VIEW
  [Tx engine\nassert · create · retract · supersede\ncascade · cardinality · unique] as TX
  [Text search + FTS5 upkeep] as TCORE
  [ObjectId codec +\nterm dictionary] as OID
  interface "Executor trait" as EXEC
}

package "Hosts" {
  [tm-rusqlite\n(bundled SQLite)] as RUSQ
  [tm-wasm\n(SQLite in WebAssembly)] as WASM
}

database "SQLite file (STRICT)" as DB {
  [term · tx · triple + indexes\nvolatile · meta] as T_GRAPH
  [term_fts · saved_answer\n(derived)] as T_DERIVED
}

SPARQL --> IR
CYPHER --> IR
API --> IR
API --> TX
API --> SAVED
API --> REVIEW
API --> IMPORT
BUDGET ..> PLAN : interrupt, row charges
SAVED --> IR : re-run with provenance
REVIEW --> TX : dry run
IMPORT --> TX : chunks
IR --> PLAN
PLAN --> SQLGEN
PLAN --> PATH
PLAN --> LFTJ
PLAN --> TEXT
SQLGEN --> VIEW
PATH --> VIEW
LFTJ --> VIEW
TEXT --> TCORE
TCORE --> VIEW
TX --> OID
TX --> TCORE
VIEW ..> EXEC
TX ..> EXEC
EXEC <|.. RUSQ
EXEC <|.. WASM
RUSQ --> DB
WASM --> DB
@enduml
```

## Agent Memory Features

The ten features of 0.3.0 answer concrete agent needs. Each reuses the views, the tx engine and the executor; the table maps it to its entry points and to the section that explains how it works.

| Feature | Agent need | Entry points | How it works |
|---|---|---|---|
| Query budgets | A tool call must not hang the agent loop | `View::with_budget`, `Db::transact_budgeted`, `QueryBudget::run`; bridge `budget` and `cancel`; MCP server flags | [[query#Query Budgets]] |
| Bulk import | Load a large memory without re-analysing after every chunk | `Db::bulk_import`; bridge `importBegin` … `importFinish`; Node and Python `BulkImport` | [[query#Bulk Import]] |
| Text recall | Find memories by words, ranked by their evidence | `View::text_search`; SPARQL `tm:textMatch`; Cypher `CALL tiramemsu.text.search`; bridge `textSearch`; MCP `text_search` | [[query#Text Recall]], [[storage#Text Index]] |
| MCP adapter | Give an LLM app typed, auditable memory tools | `tiramemsu-mcp --db file` over stdio | [[api#MCP Tools]] |
| Saved answers | Know when a remembered answer can no longer be trusted | `Db::save_answer`, `check_saved_answers`, `refresh_answer`; bridge `saveAnswer` …; MCP `save_answer`, `check_answers` | [[query#Saved Answers]], [[storage#Saved Answers]] |
| Temporal paths | Ask "could this have spread in time order" in a query | SPARQL `SERVICE <urn:tiramemsu:tm:timeRespecting…>` and `tm:arrival`; Cypher `MATCH TIME RESPECTING`; `PathArgs::time_respecting` | [[query#Temporal Path Syntax]], [[query#Physical Planning#Path Engine#Path Completeness]] |
| LFTJ operator | Cyclic patterns over skewed graphs without blow-up | `OpenOptions.planner.lftj`; `View::explain_sparql`, bridge `explainSparql` | [[query#Physical Planning#LFTJ]] |
| Conflict review and preview | See disagreement and the effect of an import before writing | `View::conflicts`, `Db::preview_bundle`; bridge `conflicts`, `previewBundle`; MCP `conflicts`, `preview_bundle` | [[query#Conflict Inspection]], [[data-model#Fact Bundles#Import Preview]] |
| Optional query frontends | Embed the store without parsers | facade cargo features `exec`, `sparql`, `cypher` | [[architecture#Crates#Cargo Features]] |
| WASM SQLite host | Run the same memory in a browser | `tm-wasm` `WasmHost`; `tiramemsu-wasm` `Database` in a Web Worker | [[architecture#WebAssembly Host]], [[bindings#WebAssembly]] |

Shared rules hold across them: every read runs in one snapshot and can be bounded by one budget; reviews and previews never write graph rows; derived state (`term_fts`, `saved_answer`) is never history and can be rebuilt or discarded; and an unsupported request fails with a typed error rather than degrading silently ([[api#Errors]]).

## Crates

The Rust workspace is split by layer so each front end compiles against the IR only, and the core never depends on a query language.

| Crate | Responsibility | Depends on |
|---|---|---|
| `tm-core` | ObjectId codec, term dictionary, SQLite schema and migrations, tx engine, views, event log, volatile table, predicate schema, the `Executor` trait, the budget meter, text search and its FTS5 upkeep, conflict inspection, bundles | nothing SQLite-specific ([[architecture#Executor]]) |
| `tm-rusqlite` | The first executor host: `rusqlite` with bundled SQLite, UDF and virtual-table registration | `rusqlite` (bundled), `tm-core` |
| `tm-wasm` | The WebAssembly host: SQLite compiled to `wasm32-unknown-unknown`, memory or OPFS storage, verified journal mode, probed capabilities ([[architecture#WebAssembly Host]]) | `tm-rusqlite`, `rusqlite` → `sqlite-wasm-rs`, `sqlite-wasm-vfs` |
| `tm-ir` | Logical algebra, semantic flags, view descriptors | `tm-core` (ids, views) |
| `tm-exec` | Planner/router, SQL codegen, path operator, the table functions `tm_path`, `tm_text` and the opt-in `tm_lftj` | `tm-ir`, `tm-core` |
| `tm-sparql` | SPARQL 1.1 (+1.2 annotations) → IR, results as SPARQL JSON/terms | `spargebra`, `tm-ir` |
| `tm-cypher` | openCypher subset + extensions → IR, results as Cypher values | a Cypher parser, `tm-ir` |
| `tiramemsu` | Facade: `Db`, `View`, `Tx`, results, reader pool, budgets, bulk import, saved answers, conflict review and previews; the only crate bindings use | `tm-core`, `tm-rusqlite`; the others behind cargo features ([[architecture#Crates#Cargo Features]]) |
| `tiramemsu-json` (`bindings/json`) | The JSON bridge: one `Database` with `call(op, args)`, shared by every binding ([[bindings#JSON Bridge]]) | `tiramemsu` with `sparql` and `cypher` |
| `tiramemsu-mcp` | Local stdio MCP server with typed memory tools ([[api#MCP Tools]]) | `tiramemsu-json`, `tiramemsu` |

Bindings (PyO3, napi-rs, WASM in `bindings/wasm`, MCP server) are separate crates on top of `tiramemsu`. The MCP server `tiramemsu-mcp` is a workspace crate under `crates/` built on the JSON bridge, so protocol code stays out of `tm-core` and the facade. See [[api#Bindings]].

`tm-core` has no normal dependency on any SQLite crate: `tm-rusqlite` appears only among its dev-dependencies, for tests. Front ends depend on `tm-ir` and `tm-core`, never on each other or on `tm-exec`; the facade wires them together.

```plantuml
@startuml crates
skinparam componentStyle rectangle
skinparam shadowing false

[tm-core] as CORE
[tm-rusqlite] as RUSQ
[tm-wasm] as WASM
[tm-ir] as IR
[tm-exec] as EXEC
[tm-sparql] as SPARQL
[tm-cypher] as CYPHER
[tiramemsu] as FACADE
[tiramemsu-json] as JSON
[tiramemsu-mcp] as MCP
[tiramemsu-node] as NODE
[tiramemsu-python] as PY
[tiramemsu-wasm] as BWASM
[rusqlite\n(bundled SQLite)] as RUSQLITE
[sqlite-wasm-rs +\nsqlite-wasm-vfs] as SWASM

RUSQ --> CORE
RUSQ --> RUSQLITE
WASM --> RUSQ
WASM --> SWASM
IR --> CORE
EXEC --> IR
SPARQL --> IR
CYPHER --> IR
FACADE --> CORE
FACADE --> RUSQ
FACADE ..> EXEC : exec
FACADE ..> SPARQL : sparql
FACADE ..> CYPHER : cypher
JSON --> FACADE : sparql + cypher
MCP --> JSON
NODE --> JSON
PY --> JSON
BWASM --> JSON
BWASM --> WASM
@enduml
```

Dotted edges are optional facade features; `sparql` and `cypher` each imply `exec`. `tm-exec`, `tm-sparql`, `tm-cypher` and `tm-wasm` also depend on `tm-core` directly; those edges are left out for clarity.

### Cargo Features

The facade's query engine and front ends are optional dependencies (`add-optional-query-frontends`): `default-features = false` links no IR, planner or parser, and the default build is unchanged.

| Feature | Facade surface | Adds to the dependency tree |
|---|---|---|
| none | `Db`, transactions, speculation, views and `triples`, dependents, bundles and their JSON and N-Triples forms, previews, conflicts, text recall, bulk import, budgets | `tm-core`, `tm-rusqlite` |
| `exec` | `execute_ir`, `explain_ir`, `View::path*`, LFTJ, the `ir` module and the engine open options | `tm-ir`, `tm-exec` |
| `sparql` | `View::sparql*`, `SparqlResult`, `Solutions`; implies `exec` | `tm-sparql`, `spargebra`, `peg` |
| `cypher` | `View::cypher`, `Db::cypher_write*`, `TxCypher`; implies `exec` | `tm-cypher`, `open-cypher` |
| `default` | `sparql` + `cypher`, and saved answers, which store queries of both languages | all |

- **Serialisation is not parsing.** The RDF terms and the N-Triples writer moved from `tm-sparql` to [[crates/tm-core/src/rdf.rs#render]] and [[crates/tm-core/src/rdf.rs#write_ntriples]], which `tm-sparql` re-exports, so bundle N-Triples and JSON need no front end.
- **No engine without `exec`.** The facade's engine type is uninhabited and [[crates/tiramemsu/src/db.rs#open_engine]] installs nothing, so a core build behaves like `query_engine: false` with the options absent.
- **Disabled APIs are absent**, not runtime errors, and the file format does not depend on the features. The JSON bridge, the MCP server and the bindings enable `sparql` and `cypher` explicitly.
- **Compatibility.** Until 0.2 the `sparql` feature only switched the SPARQL side of the differential suite and `default-features = false` still built both front ends; the facade README documents the migration.
- **Checks.** `scripts/feature-matrix.sh` asserts each combination's `cargo tree -e normal`, checks and tests it, and hands one file between it and the default build; the CI `features` job runs it for core, exec, sparql, cypher, default and the bindings ([[tests#Optional Query Frontends]]).

## Executor

`tm-core` reaches SQLite through a small synchronous `Executor` trait, so the engine can run on any host with interactive transactions. Hosts: `rusqlite` with bundled SQLite, and `tm-wasm` on SQLite compiled to WebAssembly ([[architecture#WebAssembly Host]]).

The boundary was drawn before any code existed, because it cost little then and would have cost a lot later. oxilite, which started from an abstract executor, runs on five SQLite hosts ([[prior-art#oxilite]]).

- **Required of every host:** prepared statements with bound parameters, interactive transactions (`BEGIN IMMEDIATE` … `COMMIT`/`ROLLBACK`), savepoints, and a stable snapshot within a read transaction. The tx engine reads before it writes (idempotent assert, cascade, schema checks, dictionary lookup), so it needs all of them.
- **Capabilities** (declared by the host): `reader_pool` (otherwise the reader is the writer, as on WASM), `functions` (scalar UDFs), `vtab` (virtual tables: `tm_path`, `tm_text` and `rarray`), `stat4`, `fts5` (text recall and its derived index, [[storage#Text Index]]; without it only recall fails, with `MissingCapability`).
- **Tiers:** `tm-core` needs only the required set, so a minimal host can run transactions, views, the event log and `View::triples`. `tm-exec` (SPARQL, Cypher, paths) also needs `functions` and `vtab`, and refuses to open on a host without them rather than degrading silently. A facade built without the `exec` feature is the `tm-core` tier at compile time ([[architecture#Crates#Cargo Features]]).
- **Hosts considered:** `rusqlite` (v1, all capabilities); SQLite compiled to WASM (`tm-wasm`: `functions`, `vtab` and `fts5`, no `stat4`, no `reader_pool`, probed at runtime); Cloudflare Durable Objects SQLite, which has interactive transactions through `transactionSync` but no user functions or virtual tables, so it gets the `tm-core` tier only; Turso, capabilities to be verified. Cloudflare D1 is out of scope: it has no interactive transactions (see oxilite's D5).
- `tm-exec` checks the capabilities in [[crates/tm-exec/src/host.rs#check_capabilities]] and registers its SQL functions and native operators through the executor's `registry()` hook (`HostRegistry` in `tm-core`); the `rusqlite` glue stays in `tm-rusqlite`.
- Host-specific details, such as `prepare_cached`, `Connection::from_handle` inside a virtual table, and `rarray`, stay inside the host crate.
- **Interruption** is optional: `Executor::set_interrupt` hands a host the stop conditions of a budgeted operation. The `rusqlite` host maps them to SQLite's progress handler; the default ignores them, and then only the engine's own checks stop work ([[query#Query Budgets]]).

## Connections and Concurrency

One writer connection behind a mutex and a pool of reader connections, all on one WAL-mode SQLite file.

- **Writer:** every transaction, speculative `with`, and schema change goes through the single writer. Transactions are serialised, which gives MERGE and `sys:unique` checks their atomicity. See [[time-model#Operations]].
- **Import lease:** a bulk import session holds an exclusive write lease between its chunks; other writes fail with `ImportInProgress` instead of interleaving, and readers carry on ([[query#Bulk Import]]).
- **Readers:** each query takes a reader connection and runs inside one read transaction, so it sees a consistent WAL snapshot.
- **Bounded acquisition:** waiting for a reader is unbounded by default; `OpenOptions::reader_timeout` or a per-operation budget turns a long wait into `PoolTimeout`, and a deadline or cancellation also ends it ([[query#Query Budgets]]).
- **Recovery:** returned errors, failed read commits, and unwinding callbacks roll back active snapshots. A pooled executor is returned before the original panic is resumed, so caught panics do not reduce reader capacity.
- **Historical reads** never conflict with writes: rows are only ever appended or have `t_ret` set once, and `asOf(t)` for `t` ≤ the last committed tx is stable forever. See [[time-model#Never Forget]].
- The `tx` counter is read and incremented inside the writer transaction, so `t` is gap-free and strictly increasing.

```plantuml
@startuml concurrency
skinparam shadowing false
actor "Agent A" as A
actor "Agent B" as B
participant "Db" as DB
participant "Writer\n(mutex)" as W
participant "Reader pool" as R
database "SQLite WAL" as S

A -> DB : transact(ops)
DB -> W : lock
W -> S : BEGIN IMMEDIATE
W -> S : insert / set t_ret
W -> S : COMMIT
W --> DB : TxReport{t}
B -> DB : as_of(t).cypher(q)
DB -> R : acquire
R -> S : BEGIN (snapshot)
R -> S : SELECT … (view predicates)
R --> B : rows
@enduml
```

## WebAssembly Host

`tm-wasm` runs the engine on SQLite compiled to `wasm32-unknown-unknown` inside a Web Worker, with the database in memory or in OPFS, in the native file format (`add-wasm-sqlite-host`).

- **Runtime choice.** `rusqlite` 0.40 links [`sqlite-wasm-rs`](https://crates.io/crates/sqlite-wasm-rs) on that target (SQLite 3.53 compiled to WASM, VFSes in Rust), so the host reuses the `RusqliteExec` of `tm-rusqlite` for statements, savepoints, registration and interrupts. [[crates/tm-wasm/src/lib.rs#WasmHost]] adds storage, the journal policy and the probe. Calls are synchronous, hence the worker.
- **Storage.** `Storage::Memory` (`memvfs`, any context, volatile), `Storage::Opfs` (the sync-access-handle pool of `sqlite-wasm-vfs`, dedicated worker, installed with `install_opfs`), and `Storage::File`, the native file VFS for tests. A storage the target lacks is `Unsupported`.
- **Probed capabilities** ([[crates/tm-wasm/src/probe.rs#probe_capabilities]]): each is established by running it on a scratch connection. In WebAssembly `functions`, `vtab` and `fts5` hold, so the whole query engine runs; `stat4` is not compiled in and `reader_pool` is never declared (`THREADSAFE=0`, no shared memory). `limit_capabilities` narrows the set, and the facade then refuses the engine with `MissingCapability`.
- **Journal contract.** Neither VFS has shared memory, so SQLite answers `journal_mode = WAL` with `delete`. The host reads the mode the runtime reports at open and fails with `MissingCapability` under `Journal::Wal`; `Journal::Rollback` is the explicit opt-in. [[crates/tm-wasm/src/lib.rs#WasmExec]] keeps the engine's WAL switch from changing the verified mode. The file format is identical, a hot journal is rolled back on open, and a native host switches the file back to WAL.
- **Interchange.** `export_file` and `import_file` move committed bytes; a closed native `Db` leaves no `-wal` (the readers close before the writer), so its main file is complete.
- **Limits.** `std::time::Instant` and `SystemTime` are missing on the target: `WasmClock` supplies `Date.now()`, and budgets and bulk import sessions are refused by the binding. SPARQL needs `--cfg getrandom_backend="wasm_js"` (in `.cargo/config.toml`). The OPFS tests snapshot storage during an open write; a worker killed mid-write on OPFS is not exercised.

## Deployment

The engine is a library linked into the host process; there is no server. Four ways in, one executor contract and one file format, so the same file moves between native and browser hosts ([[architecture#Executor]]).

- **Rust:** an application links `tiramemsu` and calls `Db`, `View` and `Tx` directly; cargo features choose how much of the query engine is linked ([[architecture#Crates#Cargo Features]]).
- **Node.js and Python:** the native addons call the JSON bridge in-process; values cross as JSON text ([[bindings]]).
- **Agents over MCP:** an MCP client starts `tiramemsu-mcp` as a child process and talks JSON-RPC over stdio; the server owns the file ([[api#MCP Tools]]).
- **Browser:** a page posts messages to a Web Worker that runs `tiramemsu-wasm` on `tm-wasm`, with the database in memory or OPFS and a rollback journal ([[architecture#WebAssembly Host]]).

```plantuml
@startuml deployment
skinparam shadowing false

node "Native process" {
  component "Rust app" as RUST
  component "Node.js / Python app" as SCRIPT
  component "napi-rs / PyO3 addon" as ADDON
  component "tiramemsu-json\n(JSON bridge)" as JSON
  component "tiramemsu facade" as FACADE
  component "tm-rusqlite\n(bundled SQLite, WAL)" as RUSQ
}

node "Agent host" {
  component "LLM app\n(MCP client)" as CLIENT
}

node "tiramemsu-mcp process" {
  component "MCP server\n(stdio JSON-RPC)" as MCP
  component "JSON bridge + facade" as MCPCORE
}

node "Browser" {
  component "Web page" as PAGE
  node "Web Worker" {
    component "tiramemsu-wasm\n(Database.call)" as BWASM
    component "tm-wasm\n(SQLite in WASM)" as WASM
  }
  database "OPFS or memory" as OPFS
}

file "memory.db\n(one file format)" as FILE

RUST --> FACADE
SCRIPT --> ADDON
ADDON --> JSON
JSON --> FACADE
FACADE --> RUSQ
RUSQ --> FILE
CLIENT --> MCP : stdin / stdout
MCP --> MCPCORE
MCPCORE --> FILE
PAGE --> BWASM : postMessage
BWASM --> WASM
WASM --> OPFS : rollback journal
OPFS ..> FILE : exportFile / importFile
@enduml
```

## Project Website

A static site in `site/` (a home page and articles) is published to GitHub Pages by `.github/workflows/pages.yml` on pushes to `main` that touch `site/`. There is no build step.

The custom domain is `tiramemsu.com`, set in the repository's Pages settings (the workflow deploy ignores a `CNAME` file). Its DNS records at Cloudflare must be DNS-only (grey cloud): four apex A records to GitHub's Pages IPs `185.199.108.153` to `185.199.111.153`, and `www` as a CNAME to `volland.github.io`. A proxied (orange cloud) record stops GitHub issuing the HTTPS certificate and serves "Site not found", and would also make Cloudflare a data processor that `datenschutz.html` does not name.

The pages are plain HTML with one stylesheet (`site/style.css`), light and dark palettes, and no scripts, trackers or external fonts. Links are relative, so the site works under a project path. `site/assets/` holds the icon `logo.svg` (a brain taking a bite out of a tiramisu whose layers are a graph), `layers.svg` and `architecture.svg`, which the `README.md` also uses.

- **Tone:** the slogan is "Your agent's brain loves Tiramemsu" (home page hero and `README.md`). The "What agents are saying" quotes are openly invented, say so in their intro, and each must describe a feature that exists, so a removed feature means removing its quote.
- **Legal pages:** `impressum.html`, `datenschutz.html` and `agb.html` are German, carry the operator's name, address and e-mail, and are linked from every page footer. The privacy text says the site sets no cookies and loads nothing external, and names GitHub Pages as the host that logs IP addresses, so adding a script, font, tracker or form means revising `datenschutz.html` first. They are a good-faith draft, not legal advice.
- **Articles:** `layered-graphs.html` explains statement ids and layers with queries taken from the test suite. `time-travel.html` explains the two clocks with a worked example whose queries mirror `lat.md/query#Temporal Syntax` and `crates/tiramemsu/tests/cypher_temporal.rs`, so a change to the temporal syntax or the view semantics should be reflected there. `metagraphs.html` explains edges as vertices and vertices and edges as containers, with queries from [[recipes#Metagraphs]] and the test `metagraph_containers_nesting_and_fold`, so a change to graph names, supersede or the dual view should be reflected there. `tiramemsu-vs-oxilite.html` compares the two databases and quotes the benchmark in `bench/eid-vs-reifier/`. Their numbers come from `bench/`, [[prior-art#oxilite]] and the status table, so a change to those should be reflected there.
- **Paper:** `site/paper/index.html` presents the preprint ([[paper]]) with its abstract and main results, and serves the revision-4 build as `site/paper/layered-bitemporal-graphs-v4.pdf`. `articles/grounded-memory.html` is its plain-language companion. Both say which repairs the paper describes that the released engine lacks, so a release that ships them must update both.
- **Quick start:** the home page shows the same story in Python, Node.js and Rust, as CSS-only tabs (radio inputs, no script). The Rust tab and the README snippet are trimmed from `crates/tiramemsu/examples/quickstart.rs`, which `cargo clippy --all-targets` compiles. The Python and Node.js tabs were run against the built packages, so a change to either binding's API means re-running them and updating the tabs.

## Long-Read Article

`article/` holds a Substack long read about tiramemsu with its illustrations, separate from the site. It quotes the status table, the recipes and the benchmarks, so a change to those should be reflected there.

`tiramemsu-long-read.md` is the source; `preview.html` is a self-contained copy for pasting into the Substack editor, rebuilt by `build-preview.sh` with pandoc. The illustrations are SVGs in `images/src/`, rendered to 1456 px PNGs, because Substack takes no SVG. The article avoids tables, which Substack cannot show, and `article/README.md` lists the title, captions and publishing steps.

## Crate Documentation and Publishing

Each crate's `README.md` is its crates.io page and, through `#![doc = include_str!("../README.md")]`, its crate-level rustdoc, so every Rust block in it is a doctest.

- **Links** in READMEs are absolute `https://github.com/Volland/tiramemsu/...` URLs, because relative ones break on crates.io and docs.rs. Diagrams are ASCII, since crates.io does not render Mermaid; the repository README uses Mermaid.
- **Metadata** is inherited from `[workspace.package]`: version, licence, repository, keywords, categories and `rust-version = "1.88"`, the minimum that `open-cypher` needs. Workspace path dependencies carry the workspace version (`version = "0.3.0"`) so the crates can be published. Each crate directory holds copies of both licence files.
- **Package size:** `tm-sparql` and `tm-cypher` exclude their W3C and openCypher TCK test data from the published package.
- **Publish order** follows the dependencies: `tm-core`, `tm-ir`, `tm-rusqlite`, `tm-exec`, `tm-sparql`, `tm-cypher`, `tiramemsu`, `tm-wasm`, then `tiramemsu-json` and `tiramemsu-mcp`. The Node, Python and WebAssembly binding crates are not published to crates.io.
- **Core stays SQLite-free:** doctests inside `tm-core/src` may not spell `tm_rusqlite::`, because a test greps that source for `rusqlite::` (see [[architecture#Executor]]). They use an import alias, and the README, which is not scanned, uses the normal form.
- **Checks:** `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` and `cargo test --workspace --doc` must pass.
- **Reduced builds:** the facade includes its README as crate docs only when both front ends are enabled, since the tour uses them; other combinations get a short feature summary, and doctests on core items use only the core API ([[architecture#Crates#Cargo Features]]).
