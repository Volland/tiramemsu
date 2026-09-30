# tm-cypher

openCypher front end for [Tiramemsu](https://github.com/Volland/tiramemsu): parse, check and run Cypher over a bitemporal graph in which every relationship is also a node.

## Where it sits

Tiramemsu is a layered, bitemporal graph memory on one SQLite file. Every fact is a statement `(eid, s, p, o)` with its own id, and an id can be the subject or object of another statement. SPARQL and Cypher read and write the same statements through one logical IR:

```text
  SPARQL text                 Cypher text
      |                           |
    tm-sparql                tm-cypher (this crate)
      \                          /
       +-----> tm-ir (IR) <-----+        Rust API (Db, View, Tx)
                  |                              |
              tm-exec (planner, SQL, paths) <----+
                  |
        tm-core / tm-rusqlite (one SQLite file)
```

`tm-cypher` parses with [`open-cypher`](https://crates.io/crates/open-cypher) (pinned to `=0.2.1` behind an adapter, see [PARSER.md](https://github.com/Volland/tiramemsu/blob/main/crates/tm-cypher/PARSER.md)), checks scopes, types, aggregates, parameters and writes, and then interprets the query clause by clause. Graph patterns, label tests and every time view lower to the IR; expressions, functions, projection, aggregation, ordering, `UNWIND`, `UNION` and write clauses run in Rust, because the IR has no list or map values. The crate reaches the store only through the [`Runner`] trait, so it depends on neither `tm-exec` nor SQLite.

**Most applications should use the [`tiramemsu`](https://github.com/Volland/tiramemsu/tree/main/crates/tiramemsu) facade**: `db.now().cypher(text, &params)` reads, and `db.cypher_write(opts, text, &params)` or `tx.cypher(...)` writes in one transaction. Use `tm-cypher` directly to check queries without a database (a linter, an editor integration, a query gateway), or to implement [`Runner`] for another host.

## Usage

[`compile`] parses and checks a query against a [`CompileCtx`] (vocabulary, default view, read-only or writable) and never touches a store:

```rust
use tm_cypher::{compile, CompileCtx, CypherParams, CypherError, Vocab};
use tm_ir::View;

let ctx = CompileCtx { vocab: Vocab::default(), view: View::NOW, writable: false };
let params = CypherParams::new();

let prog = compile("MATCH (p)-[r:worksAt]->(o) RETURN p, o, r.confidence", &params, &ctx)?;
assert!(!prog.has_write);

// Errors carry the byte span in the original text.
let text = "MATCH (p) RETURN q";
match compile(text, &params, &ctx) {
    Err(CypherError::Parse { span, msg }) => {
        assert_eq!(span.text(text), Some("q"));
        assert!(msg.contains('q'));
    }
    other => panic!("unexpected: {other:?}"),
}
# Ok::<(), CypherError>(())
```

Running it takes a [`Runner`]; the facade provides one. End to end, with a relationship that is also a `:Statement` node, so a layer about the fact is reachable by pattern:

```rust
use tiramemsu::{CypherParams, CypherValue, Db, OpenOptions, TxOptions};

let dir = tempfile::tempdir()?;
let db = Db::open(dir.path().join("g.db"), OpenOptions::default())?;
let p = CypherParams::new();

db.cypher_write(TxOptions::default(),
    "CREATE (a {`@id`: 'urn:tiramemsu:v:alice'})-[:worksAt {confidence: 0.8}]->\
     (c {`@id`: 'urn:tiramemsu:v:acme'})", &p)?;

// r is a relationship here ...
let r = db.now().cypher("MATCH (a)-[r:worksAt]->(c) RETURN r.confidence AS conf", &p)?;
assert_eq!(r.rows[0][0], CypherValue::Float(0.8));

// ... and the same id is a :Statement node with time metadata.
let r = db.now().cypher("MATCH (s:Statement) WHERE s.confidence IS NOT NULL RETURN s.txAdded", &p)?;
assert_eq!(r.rows.len(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Supported syntax

**Read.** `MATCH`, `OPTIONAL MATCH`, `WHERE`, `WITH`, `RETURN`, `ORDER BY`, `SKIP`, `LIMIT`, `UNWIND`, aggregates, `UNION`, `EXISTS { }` and pattern predicates, `CALL { }` subqueries (uncorrelated or importing `WITH`), named paths, variable-length relationships, `shortestPath` and `allShortestPaths` (native path engine; a variable-length pattern needs a bound endpoint), list comprehension, map projection, `CASE`, quantifiers, `reduce`, and `CALL db.labels()`, `db.relationshipTypes()`, `db.propertyKeys()`.

**Write.** `CREATE` creates; `MERGE` upserts or matches an atomic pattern; `SET` asserts or supersedes; `REMOVE` and `DELETE` retract; `DETACH DELETE` retracts every statement that mentions the node. A whole query is one transaction, and nothing is deleted from the file. `CREATE (n)` with nothing attached writes nothing, because a node exists only through its statements.

**Dual view.** A relationship variable may also stand in node position. That is how Cypher reaches layers: `MATCH (a)-[r:worksAt]->(c), (b:Belief)-[:supportedBy]->(r) RETURN a, r.confidence, b`. A statement used as a node has the implicit label `:Statement` and the properties `txAdded`, `txRetracted`, `validFrom` and `validTo`. Neo4j rejects a relationship variable in node position; everything else means what it means in Neo4j.

**Time.** `USE AS OF 150`, `USE AS OF datetime('2026-09-01T12:00:00Z')`, `USE VALID AT date('2025-03-01')` and `USE HISTORY` at the start of a query, a `UNION` branch or a `CALL { }` body. Per pattern, use `CALL { USE AS OF 150 MATCH ... RETURN ... }`. `USE GRAPH` is unsupported. A write query with a non-current top-level `USE` is unsupported.

**Names.** Labels, types and property keys map to IRIs through one `@vocab` (default `urn:tiramemsu:v:`, case kept) and a prefix table; a backticked CURIE such as `` `schema:name` `` uses the table. The vocabulary in force at compile time applies, even under `USE AS OF`. The reserved key `` `@id` `` sets or matches a node's IRI.

**Values.** Datetimes keep their offset (`datetime()` round-trips it; a named zone is stored as its offset at that instant); `localdatetime()` has none; comparisons use the instant. Integers beyond 60 bits round-trip. `CypherResult::to_json` gives a JSON form.

**Semantics.** Bag of eids, relationship isomorphism (`REPEATABLE ELEMENTS` opts out), null with three-valued logic.

**Unsupported** (fails with `Error::Unsupported`, never a wrong answer): `FOREACH`, `LOAD CSV`, `CALL { } IN TRANSACTIONS`, schema commands, quantified path patterns, GQL path modes, `!`/`&`/`%` label expressions, pattern comprehension, user-defined procedures, property maps on variable-length relationships, the bidirectional arrow `<-[]->`, `USE GRAPH`, durations, and `time`/`localtime`. The wider temporal function family (truncation, map constructors, durations, printed zone names) is deferred. Variable-length and shortest-path patterns stop at the database's `path_max_hops` (default 15).

## Conformance

The runner in `tests/tck.rs` executes the openCypher TCK (2024.3). **2 615 of 3 880 scenarios pass (67 %)**; 2 361 of the 3 562 read-only scenarios pass. Every one of the 1 265 failures is listed with a reason in [`allowlist.txt`](https://github.com/Volland/tiramemsu/blob/main/crates/tm-cypher/tests/tck/allowlist.txt), and the build fails on any unexpected failure and on any listed scenario that starts passing. Of the failures, about 1 064 are the deferred temporal family (`expressions/temporal` passes 5 of 1 004), 60 rely on empty nodes persisting (a node exists only through its statements), 50 need user-defined test procedures, 46 are dual-view cases (Neo4j rejects a relationship variable in node position and Tiramemsu accepts it), and the rest are pattern comprehension, unbound-endpoint variable-length paths, multi-valued shapes and a few analysis checks. Outside the temporal family, 2 610 of 2 876 scenarios pass (about 91 %).

## Tour

- [`compile`], [`CompileCtx`], [`CypherProgram`], [`CypherParams`]: check a query and package it.
- [`exec::run`] and [`Runner`]: interpret a program against a host; [`Rows`] is the shape of an IR result.
- [`CypherResult`], [`CypherValue`], [`NodeValue`], [`RelValue`], [`PathValue`]: what a query returns, with `to_json`.
- [`CypherError`], [`CResult`], [`Span`]: front-end errors with byte spans, converted to the facade error with [`CypherError::into_core`].
- [`Vocab`], [`VocabExt`]: name to IRI mapping.
- [`ast`], [`parse`], [`sema`]: the AST, the parser adapter (`parse::adapter::parse`) and the semantic checks, public for tools and tests.

## Stability

Version 0.1. The public API may change between minor versions, and the AST and interpreter modules are the most likely to move. The parser dependency is pinned exactly. Rust 1.88 or newer.

## License

MIT OR Apache-2.0. See [LICENSE-MIT](https://github.com/Volland/tiramemsu/blob/main/crates/tm-cypher/LICENSE-MIT) and [LICENSE-APACHE](https://github.com/Volland/tiramemsu/blob/main/crates/tm-cypher/LICENSE-APACHE). Project: <https://github.com/Volland/tiramemsu>.
