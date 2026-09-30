# tm-sparql

SPARQL 1.1 and RDF 1.2 front end for [Tiramemsu](https://github.com/Volland/tiramemsu): parse a query or update, check it, and lower it to the shared logical IR, before any SQL runs.

## Where it sits

Tiramemsu is a layered, bitemporal graph memory on one SQLite file. Every fact has an id (an `eid`), so a fact can be the subject of another fact. Two query languages share one logical IR and one executor:

```text
  SPARQL text                 Cypher text
      |                           |
  tm-sparql (this crate)      tm-cypher
      \                          /
       +-----> tm-ir (IR) <-----+        Rust API (Db, View, Tx)
                  |                              |
              tm-exec (planner, SQL, paths) <----+
                  |
        tm-core / tm-rusqlite (one SQLite file)
```

`tm-sparql` parses with [`spargebra`](https://crates.io/crates/spargebra) (Oxigraph's parser and algebra, with SPARQL 1.2 syntax on), lowers the algebra to `tm_ir::IrQuery` with the SPARQL semantic flags (homomorphism matching, unbound for missing values, set of triples), and turns updates into a plan of asserts and retracts. It does not touch a database and does not depend on SQLite.

**Most applications should use the [`tiramemsu`](https://github.com/Volland/tiramemsu/tree/main/crates/tiramemsu) facade**: `db.now().sparql(text)` prepares, runs and decodes for you, and runs updates in one transaction. Depend on `tm-sparql` directly to build another host or tool on the same front end: a linter or query explainer, an editor integration, a different executor for the IR, or a test that pins the IR a query lowers to.

## Usage

[`prepare`] parses and lowers a text against an [`Env`](env::Env), which says which view the text runs on, what `v:` means and which prefixes are declared:

```rust
use tm_ir::View;
use tm_sparql::{env::Env, lower::QueryForm, prepare, Prepared};

let env = Env::new(View::NOW);
let text = "SELECT ?who ?org WHERE { ?who v:worksAt ?org }";
let Prepared::Query(plan) = prepare(text, &env)? else { unreachable!() };

assert_eq!(plan.form, QueryForm::Select);
// The IR is the contract with the executor; it prints as text.
assert!(plan.query.to_string().contains("worksAt"));
# Ok::<(), tm_core::Error>(())
```

Unsupported constructs fail at prepare time with a named feature, never with a wrong answer:

```rust
# use tm_ir::View;
# use tm_sparql::{env::Env, prepare};
use tm_core::Error;

let env = Env::new(View::NOW);
match prepare("DESCRIBE <urn:tiramemsu:v:alice>", &env) {
    Err(Error::Unsupported { feature }) => assert_eq!(feature, "DESCRIBE"),
    other => panic!("unexpected: {other:?}"),
}
```

Running a prepared plan needs an executor. With the facade, the whole loop, including time travel through a `tm:` IRI in `FROM`, is:

```rust
use tiramemsu::{Db, OpenOptions, SparqlResult, TxOptions, Valid, Value};

let dir = tempfile::tempdir()?;
let db = Db::open(dir.path().join("g.db"), OpenOptions::default())?;
let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    Ok(())
})?;

// Updates map to assert and retract; the whole request is one transaction.
db.now().sparql("INSERT DATA { v:bob v:worksAt v:acme }")?;

// Whole-query time scope: the state after transaction 1 only.
let q = "SELECT ?p FROM <urn:tiramemsu:tm:asOf/1> WHERE { ?p v:worksAt v:acme }";
let SparqlResult::Solutions(sol) = db.now().sparql(q)? else { unreachable!() };
assert_eq!(sol.rows.len(), 1);
assert_eq!(sol.get(0, "p"), Some(&v("alice")));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Supported syntax

**Queries.** `SELECT`, `ASK`, `CONSTRUCT`; basic graph patterns, `OPTIONAL`, `FILTER`, `UNION`, `MINUS`, `BIND`, `VALUES`, subqueries, aggregates, `ORDER BY`, `LIMIT`, `OFFSET`, `EXISTS`, and property paths. A path with `*`, `+` or `?` runs on the native path engine; sequences, alternatives and inverses become joins and unions. Results are SPARQL JSON (`SELECT`, `ASK`) and N-Triples, or RDF 1.2 N-Triples when a triple term occurs (`CONSTRUCT`).

**Time.** Tiramemsu time is chosen with standard SPARQL, so the grammar is unchanged. `FROM <urn:tiramemsu:tm:asOf/150>`, `.../asOf/2026-09-01T12:00:00Z`, `.../validAt/2025-03-01` and `.../history` scope the whole query. `SERVICE <urn:tiramemsu:tm:asOf/150> { ... }` scopes one group and nests, innermost first. A `tm:` IRI as a `GRAPH` name is a parse error that points at `SERVICE`. `?r tm:txAdded ?t` reads statement time.

**Annotations.** RDF 1.2 reifiers `~ ?r`, annotations `{| ... |}` and triple terms bind to statement ids directly, so `v:alice v:worksAt v:acme {| v:confidence ?c |}` reads a layer. Deviation from RDF 1.2: the store cannot hold an unasserted triple term, so a triple term in inserted data is asserted and `rdf:reifies` is virtual, never stored.

**Named graphs.** `GRAPH <g>`, `GRAPH ?g`, `FROM` and `FROM NAMED` (non-`tm:` IRIs). A graph is a node and membership is a layer statement. The default graph is the union of all statements.

**Updates.** `INSERT DATA`, `DELETE DATA`, `DELETE/INSERT ... WHERE`, `WITH`, `USING`, `CREATE`, `CLEAR GRAPH`/`NAMED`, `DROP GRAPH`/`NAMED`. Insert is an assert, delete is a retract with cascade to layers. Deleting from a graph retracts the membership only. Updates run on the plain current view only.

**Unsupported** (fails with `Error::Unsupported { feature }`): `DESCRIBE`; `SERVICE` with a variable or a non-time IRI; negated property sets `!p`; `GRAPH ?g` over a subquery or without a triple pattern or property path; custom aggregates; `LOAD`, `ADD`, `MOVE`, `COPY`, `CLEAR`/`DROP DEFAULT` and `ALL`; updates on a non-current view; `BNODE`, `RAND`, `UUID`, `STRUUID`, `MD5`, `SHA1`, `SHA256`, `SHA384`, `SHA512`, and the SPARQL 1.2 triple functions `TRIPLE`, `SUBJECT`, `PREDICATE`, `OBJECT`; extension functions; relative IRIs (no base IRI is configured). The names are constants in [`error`].

**Known result limits.** `xsd:decimal` arithmetic and aggregates come back as `xsd:double`; `xsd:float` is not numeric; string functions drop language tags; `xsd:date` drops its timezone; numerically equal lexical forms (`01`, `1.0e0`) are one term; `AVG` over an empty group is unbound. Nodes, blank nodes, statements and transactions come back as skolem IRIs (`urn:tiramemsu:node:<n>`, `bnode:<n>`, `stmt:<n>`, `tx:<t>`) that parse back to the same ids.

## Conformance

The runner in `tests/w3c.rs` executes the W3C SPARQL 1.1 syntax, query and update tests. **634 of 781 in-scope tests pass.** The other 147 are each listed with a reason in [`expected-deviations.toml`](https://github.com/Volland/tiramemsu/blob/main/crates/tm-sparql/tests/w3c/expected-deviations.toml), and the build fails on any unlisted failure and on any listed test that starts passing. The main groups: 16 functions rejected in v1, 15 relative IRIs, about 23 reifier and triple-term differences that follow from every reifier being a stored statement, about 40 decimal, float and canonical-literal differences, 10 language-tag losses, about 10 named-graph semantics (the default graph is a union, `DROP GRAPH` retracts memberships only), and about 10 `spargebra` 0.4.7 parser quirks. Read the file for the exact list.

## Tour

- [`prepare`] and [`Prepared`]: the entry point and its two outcomes.
- [`env::Env`]: view, `@vocab`, prefixes, speculation flag and the `NOW()` instant.
- [`lower::QueryPlan`] and [`lower::QueryForm`]: the IR plus how to read its rows.
- [`update::UpdatePlan`], [`update::UpdateOp`] and [`update::run`]: a checked update and its execution against a `tm_core::Tx`.
- [`dataset`]: `tm:` time IRIs and `FROM`/`FROM NAMED` graph selection.
- [`results`]: [`results::Solutions`], RDF terms and triples, SPARQL JSON and N-Triples writers.
- [`construct::instantiate`]: `CONSTRUCT` templates over solutions.
- [`error`]: constructors and the `Unsupported` feature names.

## Stability

Version 0.1. The public API may change between minor versions. The `lower`, `update` and `dataset` internals are public for tools and tests and are the most likely to move. Rust 1.88 or newer.

## License

MIT OR Apache-2.0. See [LICENSE-MIT](https://github.com/Volland/tiramemsu/blob/main/crates/tm-sparql/LICENSE-MIT) and [LICENSE-APACHE](https://github.com/Volland/tiramemsu/blob/main/crates/tm-sparql/LICENSE-APACHE). Project: <https://github.com/Volland/tiramemsu>.
