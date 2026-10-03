# tiramemsu

**Layered, never-forget memory for agents, as an embedded Rust library on one SQLite file.**

Tiramemsu is a graph database in which every fact is a row `(eid, s, p, o)` with its own id, and that id can be the subject or object of another statement. Provenance, confidence and beliefs are therefore just more statements (layers), and every change is kept together with when the database learned it and when it was true in the world. You query the same store with SPARQL 1.1 (with RDF 1.2 annotations) or openCypher, and walk paths across layers.

This is the facade crate: the only one an application depends on. It re-exports the transaction engine (`tm-core`), the query planner (`tm-exec`), both query front ends and the rusqlite host.

## Why layered, never-forget memory

A knowledge graph stores facts. An agent also has to say how sure it is, where it read something, what it concludes from it, and later "I was wrong". Those are statements about statements, and a store that overwrites in place cannot answer "what did I believe last Tuesday, and why". Tiramemsu keeps every statement, ever, and retracts instead of deleting: a correction closes the old row and inserts a new one, replaying the layers that hung on it. `as_of` then shows the past exactly, and `history` shows how beliefs changed.

## Install

```sh
cargo add tiramemsu
```

Version 0.1, MSRV 1.88. SQLite is bundled (through `rusqlite`), so there are no other system dependencies. Storage is one file in WAL mode.

## Tour

Every snippet below is a doctest that runs against a temporary database. Values are built with `Value::iri`, and the helper `v("alice")` used throughout means `urn:tiramemsu:v:alice`, the default vocabulary that SPARQL and Cypher also resolve bare names against.

### 1. Open a database

`Db::open` creates the file (or migrates an older one) and starts one writer and a pool of readers.

```rust
use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
let db = Db::open(dir.path().join("memory.db"), OpenOptions::default())?;
assert_eq!(db.reader_count(), 4);
assert!(db.now().triples(None, None, None)?.is_empty());
# Ok::<(), Error>(())
```

### 2. Write facts

All writes happen in `Db::transact`, which is one transaction with one number `t`. `assert` is idempotent, `create` always inserts (parallel edges), and `meta` puts a statement on the transaction itself, such as who wrote it. If the closure returns an error nothing is committed.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let report = db.transact(TxOptions::default(), |tx| {
    let first = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    assert!(first.is_new());
    // Asserting the same live fact again returns the existing id.
    assert!(!tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.is_new());
    // A parallel edge: create never merges.
    tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
    tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
    tx.meta(v("author"), v("agent7"))?;
    Ok(())
})?;
assert_eq!(report.t, TxId(1));
// Three facts and the metadata statement are all statements.
assert_eq!(report.asserted.len(), 4);
assert_eq!(db.now().triples(None, None, None)?.len(), 4);
# Ok::<(), Error>(())
```

The `TxReport` lists what was asserted, what already existed, and everything retracted or superseded (with its `RetKind`). Pass `TxOptions { dry_run: true, .. }` to run with full semantics, get the report and discard the effects.

### 3. Layers

An assert returns the fact's `Eid`. Wrap it in `Value::Stmt` and use it as a subject or object to say something about the fact. A belief can in turn be supported by the fact.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    let fact = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    let conf = Value::literal("0.8", Some("http://www.w3.org/2001/XMLSchema#double"), None);
    tx.assert(Value::Stmt(fact), v("confidence"), &conf, Valid::ALWAYS)?;
    tx.assert(Value::Stmt(fact), v("source"), Value::str("chat-2026-09-29"), Valid::ALWAYS)?;
    tx.assert(v("belief9"), v("supportedBy"), Value::Stmt(fact), Valid::ALWAYS)?;
    Ok(())
})?;
assert_eq!(db.now().triples(None, None, None)?.len(), 4);
# Ok::<(), Error>(())
```

### 4. Correct with `supersede`

`supersede(eid, patch)` is the general update verb. It retracts the fact and everything layered on it, then re-inserts the whole structure under new ids with the patch applied to the root. A `Patch` can change the object and the valid-time bounds, never the subject or predicate (that would be a different fact). The new root gets a `sys:supersedes` link to the old one.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let mut fact = None;
db.transact(TxOptions::default(), |tx| {
    let e = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    tx.assert(Value::Stmt(e), v("source"), Value::str("chat"), Valid::ALWAYS)?;
    fact = Some(e);
    Ok(())
})?;
let report = db.transact(TxOptions::default(), |tx| {
    tx.supersede(fact.unwrap(), Patch::object(v("globex")))?;
    Ok(())
})?;
// Two statements were replaced (the fact and its source layer), none deleted.
assert_eq!(report.superseded.len(), 2);
let now = db.now().sparql("SELECT ?o WHERE { v:alice v:worksAt ?o }")?;
assert_eq!(now.solutions().unwrap().rows.len(), 1);
# Ok::<(), Error>(())
```

For a predicate flagged cardinality-one (see [Predicate schema](#predicate-schema)), asserting a new object is retract plus assert instead: the annotations of the old value do not carry over, because the source of "30" says nothing about "31".

### 5. Confirm

`confirm(eid)` records that another source agrees, as a `sys:confirmedBy` statement pointing at the current transaction, so the transaction's own metadata says who confirmed. `assert_with(.., AssertOpts { on_existing: OnExisting::Confirm, .. })` does it when an assert finds the fact already there.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let fact = db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    Ok(())
})?.asserted[0];
db.transact(TxOptions::default(), |tx| {
    tx.meta(v("source"), v("newsletter"))?;
    tx.confirm(fact)?;
    Ok(())
})?;
let rows = db.now().sparql(
    "SELECT ?t WHERE { v:alice v:worksAt v:acme {| sys:confirmedBy ?t |} }",
)?;
assert_eq!(rows.solutions().unwrap().rows.len(), 1);
# Ok::<(), Error>(())
```

### 6. Retract, with cascade

`retract(eid)` ends a statement and, in the same transaction, every live statement that has that eid as subject or object, recursively. Only statement ids cascade, never nodes: retracting `alice worksAt acme` leaves every other fact about Alice and Acme alone. The reason belongs on the transaction (`tx.meta(sys:reason, ..)`), not on the retracted fact.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let mut fact = None;
db.transact(TxOptions::default(), |tx| {
    let e = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    tx.assert(Value::Stmt(e), v("source"), Value::str("chat"), Valid::ALWAYS)?;
    tx.assert(v("alice"), v("livesIn"), v("kyiv"), Valid::ALWAYS)?;
    fact = Some(e);
    Ok(())
})?;
let report = db.transact(TxOptions::default(), |tx| {
    tx.retract(fact.unwrap())?;
    Ok(())
})?;
// The fact and its source layer are retracted; the livesIn fact is untouched.
assert_eq!(report.retracted.len(), 2);
assert_eq!(report.retracted[1].1, RetKind::Cascade);
assert_eq!(db.now().sparql("ASK { v:alice v:worksAt v:acme }")?, SparqlResult::Boolean(false));
assert_eq!(db.now().sparql("ASK { v:alice v:livesIn v:kyiv }")?, SparqlResult::Boolean(true));
# Ok::<(), Error>(())
```

### 7. Time travel

Transaction time answers "what did the database believe", valid time answers "what was true". `Db::now()`, `as_of(TimeRef)` (a transaction number or a wall-clock instant) and `history()` return a `View`, a cheap immutable value; `View::valid_at(ms)` narrows any of them to statements whose valid interval contains an instant. Valid intervals are half open, `[from, to)`, in epoch milliseconds.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
// Alice worked at Acme during [1000, 2000).
let mut fact = None;
let t1 = db.transact(TxOptions::default(), |tx| {
    fact = Some(tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::between(1000, 2000))?.eid());
    Ok(())
})?;
// Later we learn she stayed until 3000: a correction, not an overwrite.
let patch = Patch { v_to: Some(Some(3000)), ..Patch::default() };
let t2 = db.transact(TxOptions::default(), |tx| {
    tx.supersede(fact.unwrap(), patch)?;
    Ok(())
})?;

let ask = "ASK { v:alice v:worksAt v:acme }";
let yes = SparqlResult::Boolean(true);
// Believed at tx 1: not valid at 2500. Believed now: valid at 2500.
assert_ne!(db.as_of(TimeRef::Tx(t1.t.0)).valid_at(2500).sparql(ask)?, yes);
assert_eq!(db.now().valid_at(2500).sparql(ask)?, yes);
assert_eq!(db.now().valid_at(3000).sparql(ask)?, SparqlResult::Boolean(false)); // half open
// The history keeps both rows, with their real lifetimes.
let all = db.history().triples(None, None, None)?;
assert_eq!(all.iter().filter(|t| t.t_ret == Some(t2.t)).count(), 1);
assert_eq!(all.iter().filter(|t| t.t_ret.is_none()).count(), 2); // the new fact and sys:supersedes
# Ok::<(), Error>(())
```

Rows read through an `as_of` view report `t_ret` and `ret_kind` as `None`, because any retraction visible there happened after the view's transaction. Use `history()` for real lifetimes. `Db::events_since(t)` returns the change log after `t`. `as_of(TimeRef::Instant(ms))` resolves to the last transaction at or before that time. Transaction instants are strictly increasing, even when the clock steps backwards.

### 8. Speculate

`Db::with(ops, query)` applies operations hypothetically on the writer, runs your query against a `View` that sees them, and discards everything. `TxOptions { dry_run: true, .. }` does the same for a whole transaction and hands back the report, which is how you preview a cascade. Neither consumes a transaction number nor writes history. Both hold the write lock, so keep them short.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let mut fact = None;
db.transact(TxOptions::default(), |tx| {
    let e = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    tx.assert(Value::Stmt(e), v("source"), Value::str("chat"), Valid::ALWAYS)?;
    fact = Some(e);
    Ok(())
})?;
let fact = fact.unwrap();

// What would remain if I retracted it?
let left = db.with(
    |tx| { tx.retract(fact)?; Ok(()) },
    |view| Ok(view.triples(None, None, None)?.len()),
)?;
assert_eq!(left, 0);

// How much would a retraction take with it?
let preview = db.transact(TxOptions { dry_run: true, ..TxOptions::default() }, |tx| {
    tx.retract(fact)?;
    Ok(())
})?;
assert_eq!(preview.retracted.len(), 2);

// Neither left a trace.
assert_eq!(db.now().triples(None, None, None)?.len(), 2);
assert_eq!(db.events_since(0)?.len(), 2); // only the original two asserts
# Ok::<(), Error>(())
```

### 9. SPARQL

`View::sparql` runs SPARQL 1.1 with RDF 1.2 (`SELECT`, `ASK`, `CONSTRUCT`, and updates on the current view). The prefixes `v:` (the default vocabulary), `sys:`, `tm:`, `rdf:`, `rdfs:` and `xsd:` are predeclared. The result is a `SparqlResult`: `Solutions`, `Boolean`, `Graph`, or `Update(TxReport)`, with `write_sparql_json` and `write_ntriples` for the wire formats.

A fact's id is reachable with RDF 1.2 annotation syntax. `{| ... |}` after a triple reads the layers on it, and `~ ?r` names the fact id:

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.now().sparql("INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.8 ; v:source 'chat' |} }")?;

let r = db.now().sparql(
    "SELECT ?c ?s WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ; v:source ?s |} }",
)?;
let sol = r.solutions().unwrap();
assert_eq!(sol.rows.len(), 1);
assert_eq!(sol.get(0, "s"), Some(&Value::str("chat")));

// The same layers through the fact's id.
let r = db.now().sparql("SELECT ?c WHERE { v:alice v:worksAt v:acme ~ ?r . ?r v:confidence ?c }")?;
assert_eq!(r.solutions().unwrap().rows.len(), 1);
println!("{}", r.write_sparql_json()?);
# Ok::<(), Error>(())
```

Time is per query or per group. `FROM <urn:tiramemsu:tm:asOf/150>` (a transaction number or an RFC 3339 instant), `.../history` and `.../validAt/<d>` set it for the whole query, and `SERVICE <urn:tiramemsu:tm:asOf/…> { … }` sets it for one group, so a single query can compare before and after:

```sparql
SELECT ?before ?after WHERE {
  SERVICE <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?before }
  v:alice v:worksAt ?after
}
```

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let mut fact = None;
db.transact(TxOptions::default(), |tx| {
    fact = Some(tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid());
    Ok(())
})?; // tx 1
db.transact(TxOptions::default(), |tx| {
    tx.supersede(fact.unwrap(), Patch::object(v("globex")))?;
    Ok(())
})?; // tx 2
let r = db.now().sparql(
    "SELECT ?before ?after WHERE {
       SERVICE <urn:tiramemsu:tm:asOf/1> { v:alice v:worksAt ?before }
       v:alice v:worksAt ?after }",
)?;
let sol = r.solutions().unwrap();
assert_eq!(sol.get(0, "before"), Some(&v("acme")));
assert_eq!(sol.get(0, "after"), Some(&v("globex")));
# Ok::<(), Error>(())
```

### 10. Cypher

`View::cypher` runs read-only openCypher (a write clause fails with `Unsupported`). `Db::cypher_write` runs one query that may write, in exactly one transaction, and returns the rows with the `TxReport`. Inside your own `transact` closure, the `TxCypher` extension trait gives `tx.cypher(..)`, so Cypher and the Rust operations share a transaction. Bare labels, relationship types and property keys resolve against the `@vocab` (default `urn:tiramemsu:v:`). A relationship is also a `:Statement` node, and a fact's layers read as relationship properties.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
let none = CypherParams::default();
let out = db.cypher_write(
    TxOptions::default(),
    "CREATE (:Person {name: 'Alice'})-[:WORKS_AT {confidence: 0.8}]->(:Org {name: 'Acme'})",
    &none,
)?;
assert!(out.report.is_some());

let r = db.now().cypher(
    "MATCH (p:Person)-[r:WORKS_AT]->(o:Org) RETURN p.name AS who, o.name AS org, r.confidence AS c",
    &none,
)?;
assert_eq!(r.columns, ["who", "org", "c"]);
assert_eq!(r.rows[0][0], CypherValue::String("Alice".into()));
assert_eq!(r.rows[0][2], CypherValue::Float(0.8));

// Writes inside a transaction you control.
db.transact(TxOptions::default(), |tx| {
    tx.cypher("MATCH (p:Person {name: 'Alice'}) SET p.title = 'CTO'", &CypherParams::default())?;
    Ok(())
})?;
# Ok::<(), Error>(())
```

Cypher `DELETE` retracts, and `DELETE n` on a node that still has live relationships fails with `DeleteConnectedNode`; use `DETACH DELETE`.

### 11. Paths

`View::path(start, path, mode, max_hops)` walks a SPARQL 1.1 property path (plus `{m,n}`) from a node with a native automaton search. `PathMode` is `Reachability` (endpoints only), `Trail` (no fact id repeated), `AnyShortest` or `AllShortest`. `max_hops` bounds every mode (`u32::MAX` for none). A search that exceeds `OpenOptions::path_max_states` fails with `PathLimitExceeded` and is never silently truncated. The same engine backs SPARQL property paths, Cypher variable-length patterns and the SQL table function `tm_path`.

Layers are part of the graph, so a path can cross them. Every fact id has two virtual hops, `sys:subject` and `sys:object`, that lead to its endpoints:

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
    tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)?;
    let fact = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    tx.assert(v("belief9"), v("supportedBy"), Value::Stmt(fact), Valid::ALWAYS)?;
    Ok(())
})?;
let view = db.now();
let ends = |start: &str, path: &str, mode| -> Result<Vec<Value>> {
    let s = view.encode(&v(start))?.unwrap();
    let mut out = Vec::new();
    for row in view.path(s, path, mode, u32::MAX)? {
        out.push(view.decode(row.end)?);
    }
    Ok(out)
};
assert_eq!(ends("a", "knows+", PathMode::Reachability)?, [v("b"), v("c")]);
// From a belief, through the fact it rests on, to that fact's subject and object.
let mut layered = ends("belief9", "supportedBy/(sys:subject|sys:object)", PathMode::Reachability)?;
layered.sort_by_key(|x| x.to_string());
assert_eq!(layered, [v("acme"), v("alice")]);
# Ok::<(), Error>(())
```

In SPARQL the same walk is `v:belief9 v:supportedBy/(sys:subject|sys:object)+ ?x`, and Cypher writes it as ``(b:Belief)-[:SUPPORTED_BY|`sys:subject`*2]->(x)``.

### 12. Named graphs

A graph is a node, and membership is one more layer statement `(fact sys:inGraph graph)`. There is no extra column or table. Sessions, sources, tenants or scratch spaces all fit. SPARQL supports `GRAPH`, `FROM`, `FROM NAMED`, `WITH` and graph management updates. In Rust, `Tx::add_to_graph`, `remove_from_graph`, `clear_graph`, `create_graph` and `drop_graph` do the same, and `View::graphs` and `View::graph_members` list them. Removing a membership retracts only the membership; the fact stays.

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let session = v("session12");
let mut fact = None;
db.transact(TxOptions::default(), |tx| {
    let e = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    tx.add_to_graph(e, &session, AssertOpts::default())?;
    fact = Some(e);
    Ok(())
})?;
db.now().sparql("INSERT DATA { GRAPH v:session13 { v:bob v:worksAt v:acme } }")?;

let q = "SELECT ?who WHERE { GRAPH v:session12 { ?who v:worksAt ?o } }";
assert_eq!(db.now().sparql(q)?.solutions().unwrap().rows.len(), 1);
assert_eq!(db.now().graphs()?.len(), 2);

// Leave the session: the membership is retracted, the fact is not.
db.transact(TxOptions::default(), |tx| {
    assert!(tx.remove_from_graph(fact.unwrap(), &session)?);
    Ok(())
})?;
assert!(db.now().sparql(q)?.solutions().unwrap().rows.is_empty());
assert_eq!(db.now().sparql("ASK { v:alice v:worksAt v:acme }")?, SparqlResult::Boolean(true));
# Ok::<(), Error>(())
```

A graph name that is a literal, statement or transaction fails with `InvalidGraphName`. A `GRAPH` block inside `SERVICE <urn:tiramemsu:tm:asOf/…>` is read in the group's time, so time and graphs combine.

### 13. Impact, citations, hand-offs and journeys

`dependents` lists what retracting a fact would take with it, on any view and without the write lock. `sparql_with` can cite the statements behind every row. `bundle` packs a fact with its layers and evidence for another database, and `path_with` walks only forward in valid time. More in the [recipes](https://github.com/Volland/tiramemsu/blob/main/lat.md/recipes.md).

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let other = Db::open(dir.path().join("other.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
let mut job = None;
db.transact(TxOptions::default(), |tx| {
    let e = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
    tx.assert(e, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
    tx.assert(v("belief9"), v("supportedBy"), e, Valid::ALWAYS)?;
    tx.assert(v("a"), v("met"), v("b"), Valid::between(1, 5))?;
    tx.assert(v("b"), v("met"), v("c"), Valid::between(0, 2))?; // still true when the walk reaches b at 1
    job = Some(e);
    Ok(())
})?;
let job = job.unwrap();

// impact: the fact, its confidence and the belief resting on it
assert_eq!(db.now().dependents(job)?.len(), 3);

// citations: the statement ids behind each row
let r = db.now().sparql_with(
    "SELECT ?c WHERE { v:alice v:worksAt ?c }",
    &SparqlOptions { provenance: true },
)?;
assert_eq!(r.solutions().unwrap().provenance(0), Some(&[job][..]));

// hand-off: the fact and everything about it, into another database
let bundle = db.now().bundle(job)?;
other.transact(TxOptions::default(), |tx| tx.import_bundle(&bundle).map(|_| ()))?;
assert_eq!(other.now().sparql("ASK { v:alice v:worksAt v:acme {| v:confidence ?c |} }")?,
           SparqlResult::Boolean(true));

// journeys: b is reached at 1, and b met c until 2, so c is reachable
let a = db.now().encode(&v("a"))?.unwrap();
let args = PathArgs { time_respecting: Some(TimeRespecting { after: None }), ..PathArgs::default() };
assert_eq!(db.now().path_with(a, "met+", &args)?.len(), 2);
# Ok::<(), Error>(())
```

### Predicate schema

A predicate carries optional flags, written as ordinary statements about it, so the schema itself is versioned:

| Flag | Statement | Effect |
|---|---|---|
| Cardinality one | `(p sys:cardinality sys:one)` | Asserting a new object retracts live objects with overlapping valid time; their layers are cascaded away |
| Unique | `(p sys:unique true)` | At most one live subject per object; `Tx::upsert(p, o)` returns the holder or creates it |
| Value type | `(p sys:valueType xsd:integer)` | The object must be of that type (or a `sys:<TAG>` such as `sys:IRI`) |

```rust
# use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
# let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("age"), Value::iri(vocab::SYS_CARDINALITY), Value::iri(vocab::SYS_ONE), Valid::ALWAYS)?;
    tx.assert(v("age"), Value::iri(vocab::SYS_VALUE_TYPE), Value::iri("http://www.w3.org/2001/XMLSchema#integer"), Valid::ALWAYS)?;
    tx.assert(v("email"), Value::iri(vocab::SYS_UNIQUE), Value::Bool(true), Valid::ALWAYS)?;
    Ok(())
})?;
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("age"), Value::Int(30), Valid::ALWAYS)?;
    tx.assert(v("alice"), v("age"), Value::Int(31), Valid::ALWAYS)?; // retracts 30
    tx.assert(v("alice"), v("email"), Value::str("a@example.org"), Valid::ALWAYS)?;
    Ok(())
})?;
assert_eq!(db.now().sparql("SELECT ?a WHERE { v:alice v:age ?a }")?.solutions().unwrap().rows.len(), 1);

let wrong_type = db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("age"), Value::str("old"), Valid::ALWAYS)?;
    Ok(())
});
assert!(matches!(wrong_type, Err(Error::ValueTypeMismatch { .. })));
let taken = db.transact(TxOptions::default(), |tx| {
    tx.assert(v("bob"), v("email"), Value::str("a@example.org"), Valid::ALWAYS)?;
    Ok(())
});
assert!(matches!(taken, Err(Error::UniqueViolation { .. })));
# Ok::<(), Error>(())
```

A schema change that live data already violates fails with `SchemaConflict`.

## Errors

Every failure is a typed `Error` (`#[non_exhaustive]`, so keep a wildcard arm), and a failed transaction leaves no trace: no transaction row, statements or terms. The ones you will meet:

| Error | When |
|---|---|
| `UniqueViolation`, `ValueTypeMismatch` | An assert breaks a `sys:unique` or `sys:valueType` flag |
| `SchemaConflict` | A schema flag is added that live data already violates |
| `NotLive(eid)` | `supersede`, `confirm` or `add_to_graph` on a retracted or unknown fact |
| `InvalidPatch`, `InvalidInterval` | A patch that changes `s` or `p`, changes nothing, or an empty valid interval (`from >= to`) |
| `CascadeLimitExceeded` | A retract or supersede would touch more than `TxOptions::max_cascade` (default 10 000) statements |
| `SelfReference` | A fact would use its own id as subject or object |
| `ReservedNamespace` | User data writes `sys:` or `tm:` predicates the engine owns (`sys:inGraph`, `sys:confirmedBy`, ...) |
| `InvalidGraphName`, `GraphNotFound`, `GraphExists` | A non-node graph name; `CLEAR`/`DROP`/`CREATE GRAPH` on a missing or declared graph |
| `Parse { dialect, span, msg }`, `Unsupported { feature }` | Query text is invalid, or is outside the supported subset (a write in `View::cypher`, a SPARQL update on a non-current view) |
| `Eval { dialect, msg }` | A runtime error inside a query (type error, division by zero) |
| `DeleteConnectedNode` | Cypher `DELETE` of a node that still has relationships |
| `PathLimitExceeded` | A path search passed `path_max_states` |
| `DeadlineExceeded`, `Cancelled` | A budgeted operation ran past its `timeout` or its `CancelToken` was cancelled; a write rolls back |
| `PoolTimeout` | No reader became free within the reader timeout |
| `ResultLimitExceeded { limit }` | A budgeted operation decoded more than `max_rows` rows or `max_bytes` bytes; no partial result |
| `Reentrant` | A write or speculation started inside another on the same `Db` and thread |
| `FormatVersion`, `ForeignFile` | The file is from a newer format, or is some other SQLite database |
| `IdSpaceExhausted { kind }` | A node, blank node, statement or transaction counter passed 2⁴⁸ − 1 (about 2.8 × 10¹⁴ ids per kind) |
| `Sqlite(_)`, `Custom(_)` | A SQLite failure with its result code (busy, I/O), or your own error returned from a transaction body |

## Open options

`OpenOptions::default()` is right for most uses. The ones worth knowing:

| Option | Default | Meaning |
|---|---|---|
| `readers` | 4 | Read-only connections in the pool |
| `clock` | system clock | Source of transaction instants; inject a `ManualClock` for tests |
| `busy_timeout` | 5 s | How long to wait on a SQLite lock |
| `optimize_every` | 1000 | Run `PRAGMA optimize` every this many commits (and after a commit that big) |
| `path_max_hops` | 15 | Hop cap of unbounded Cypher path patterns and `TRAIL` in `tm_path` |
| `path_max_states` | 1 000 000 | Search-state budget per path evaluation; exceeding it is an error |
| `reader_timeout` | `None` | How long a read waits for a free reader before `PoolTimeout`; `None` waits without limit |
| `term_cache_capacity`, `planner`, `query_engine` | 16 384, defaults, true | Term cache size, planner routing, and the switch for opening the storage tier only |

```rust
use std::sync::Arc;
use tiramemsu::*;
# let dir = tempfile::tempdir().unwrap();
let clock = Arc::new(ManualClock::new(1_000));
let db = Db::open(
    dir.path().join("m.db"),
    OpenOptions { readers: 2, clock: clock.clone(), ..OpenOptions::default() },
)?;
let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
    Ok(())
})?;
// With a manual clock the instant of every transaction is known.
let then = db.as_of(TimeRef::Instant(1_000));
assert_eq!(then.triples(None, None, None)?.len(), 1);
assert!(db.as_of(TimeRef::Instant(999)).triples(None, None, None)?.is_empty());
# Ok::<(), Error>(())
```

## Query budgets

Every call is unbounded by default. A `QueryBudget` bounds one operation: `timeout` (waiting, SQL and path search), `cancel` (a `CancelToken` you can cancel from any thread), `reader_timeout` (the wait for a pooled connection, separate from SQLite's `busy_timeout`), and `max_rows` / `max_bytes` (everything the operation decodes, across all its statements). A stopped read releases its connection and a stopped write commits nothing; an over-budget result is an error, never a silent prefix.

```rust
# use tiramemsu::*;
# use std::time::Duration;
# let dir = tempfile::tempdir().unwrap();
# let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
db.now().sparql("INSERT DATA { v:alice v:worksAt v:acme }")?;
let budget = QueryBudget {
    timeout: Some(Duration::from_millis(500)),
    max_rows: Some(10_000),
    ..Default::default()
};
let rows = db.now().with_budget(&budget).sparql("SELECT ?o WHERE { v:alice v:worksAt ?o }")?;
assert_eq!(rows.solutions().unwrap().rows.len(), 1);
// writes take a budget too, and a stopped one leaves no trace
let token = CancelToken::new();
token.cancel();
let stopped = QueryBudget { cancel: Some(token), ..Default::default() };
let r = db.transact_budgeted(TxOptions::default(), &stopped, |_tx| Ok(()));
assert!(matches!(r, Err(Error::Cancelled)));
# Ok::<(), Error>(())
```

`QueryBudget::run(|| ...)` bounds a sequence of calls as one operation, and `Db::cypher_write_budgeted` bounds a Cypher write.

## Concurrency

There is one writer connection behind a mutex and a pool of read-only connections on the same WAL file. `transact` and `with` serialize on the writer, so writes are atomic and never race. Reads take a pooled connection and one read snapshot, so readers do not block the writer or each other, and a `View` sees a consistent committed state. `Db` is `Send + Sync`: share it in an `Arc` across threads. Calling `transact` from inside a running `transact` or `with` on the same thread returns `Error::Reentrant` instead of deadlocking.

## The never-forget invariant

No code path deletes a statement, a term or a transaction. The only change to a stored row is setting `t_ret` and `ret_kind` once, from NULL, and SQLite triggers in the file itself reject `DELETE` and any second change, so a bug or another tool opening the file cannot break it. Consequences:

- `as_of(t)` is exact for every `t`, forever, and eids are never reused.
- Forgetting means retracting. The fact leaves `now()` and stays in `as_of` and `history`.
- The file only grows. High-churn state that should not keep history belongs in the `volatile` table (`Tx::set_volatile`), which is not part of the graph.
- Legal erasure is planned as crypto-shredding (destroy a key, keep the rows) and is not implemented; format 1 rejects `sys:sensitive` and sealed values.

## What it is not

- Not a server. It is a library in your process; there is no network protocol, MCP server or Python, Node or WASM binding yet.
- Not portable across SQLite hosts yet. The only host is rusqlite (`RusqliteHost`); Cloudflare D1 is unsupported because the engine reads before it writes. `Db::open_with_host` takes any `Host` implementation.
- Not multi-writer. One writer at a time, by design.
- Not a reasoner or validator: no RDFS/OWL inference and no SHACL. Predicate flags are the whole schema.

## Known limits

- As-of lookups slow down when one key collects many updates.
- Named-graph membership is one row per statement per graph, which roughly doubles the file when every statement is in a graph.
- SPARQL decimals come back as doubles.
- Path patterns inside `GRAPH` are unsupported.
- Cypher temporal types, `CALL` and a few dual-view cases are deferred.

## Conformance

634 of the 781 in-scope W3C SPARQL tests and 2 615 of the 3 880 openCypher TCK scenarios pass. Every other one is listed with a reason and an unexpected result fails the build.

## Examples

Runnable, and printing deterministic output. Run one with `cargo run -p tiramemsu --example <name>`.

| Example | Shows |
|---|---|
| [`quickstart`](https://github.com/Volland/tiramemsu/blob/main/crates/tiramemsu/examples/quickstart.rs) | A fact with layers, a correction, time travel, SPARQL and Cypher |
| [`time_travel`](https://github.com/Volland/tiramemsu/blob/main/crates/tiramemsu/examples/time_travel.rs) | Valid time, `supersede`, `as_of`, `history` and `valid_at` |
| [`layers_and_paths`](https://github.com/Volland/tiramemsu/blob/main/crates/tiramemsu/examples/layers_and_paths.rs) | A belief supported by a fact with layers, an annotation query and a path across layers |
| [`named_graphs`](https://github.com/Volland/tiramemsu/blob/main/crates/tiramemsu/examples/named_graphs.rs) | Sessions as graphs, `GRAPH` queries and a membership-only delete |

## Links

- Repository: <https://github.com/Volland/tiramemsu>
- Design (architecture, data model, time model, query): <https://github.com/Volland/tiramemsu/tree/main/lat.md>
- Requirements and scenarios: <https://github.com/Volland/tiramemsu/tree/main/openspec/specs>
- Layered graphs, explained: <https://github.com/Volland/tiramemsu/blob/main/site/articles/layered-graphs.html>
- Tiramemsu vs oxilite: <https://github.com/Volland/tiramemsu/blob/main/site/articles/tiramemsu-vs-oxilite.html>

## License

MIT OR Apache-2.0.
