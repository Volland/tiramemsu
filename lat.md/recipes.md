# Recipes

Questions about memory answered with statement ids, layers, transaction metadata, paths and per-pattern time scopes. Each recipe is a tested query, and most need no special feature.

Each part exists in some other system: edge annotations in RDF-star, transactions as entities in Datomic, bitemporal rows in XTDB, validity windows in Graphiti. What is unusual here is that they meet in one statement. Every statement and every layer on it has an id, both clocks, a structural link to what it annotates, and a row that is never deleted. So a single query can walk from a belief to its evidence, rewind that walk to an earlier transaction, and ask who wrote each step and why. See [[data-model#Layers]] and [[time-model]].

The tests are in `crates/tiramemsu/tests/recipes.rs` unless noted.

## Impact Analysis

Everything that retracting a statement would take with it, read without the writer lock: `View::dependents(eid)` walks the same cascade as a retraction, on any view ([[time-model#Cascade#Dependents]]).

```rust
let at_risk = db.now().dependents(e1)?;                  // what a retraction of e1 would take
let then    = db.as_of(TimeRef::Tx(150)).dependents(e1)?; // what depended on it back then
```

The same set is a path query, because the inverse virtual hops step from a statement to the statements about it:

```sql
SELECT "end" FROM tm_path(:e1, '(^sys:subject|^sys:object)*', 'REACH')
```

A property test checks that the dependents, the `retracted` list of a dry-run retraction and the ends of that path are the same set, over random layered graphs with reference cycles. `dry_run` still gives the exact report of a write, but it holds the single writer.

## Evidence Chains Through Time

A path that starts at a belief and crosses into the statements it rests on, read as of any earlier transaction. See [[query#Physical Planning#Path Engine]] and [[query#Temporal Syntax]].

```sparql
SELECT ?src WHERE {
  SERVICE <urn:tiramemsu:tm:asOf/150> {
    v:belief9 v:supportedBy/v:derivedFrom* ?src } }
```

- `v:supportedBy` has a statement as its object, so the path lands on an eid and continues through `v:derivedFrom` links between statements.
- The virtual hops `sys:subject`, `sys:object` and `sys:predicate` step from a statement to its parts: `v:belief9 v:supportedBy/sys:subject ?who` gives the person the supporting fact is about.
- Under `asOf`, the walk sees the links as they were then, including links retracted since. Neo4j cannot point an edge at an edge, RDF-star stores cannot run property paths through quoted triples, and Datomic datoms have no identity to walk to.

## Provenance From Transaction Metadata

Who wrote a fact, and why it was retracted, with no annotation on the fact itself. The transaction is a node, and its metadata is ordinary triples ([[time-model#Transaction Time]]).

```sparql
# every employment fact written by agent7
SELECT ?who ?c WHERE { ?who v:worksAt ?c ~ ?r . ?r tm:txAdded ?t . ?t sys:author v:agent7 }

# why facts were forgotten
SELECT ?who ?why FROM <urn:tiramemsu:tm:history> WHERE {
  ?who v:worksAt ?c ~ ?r . ?r tm:txRetracted ?t . ?t sys:reason ?why }
```

`Tx::meta` writes `sys:author`, `sys:source` and `sys:reason` on the current transaction. `tm:txAdded` and `tm:txRetracted` are virtual predicates, so the hop from a statement to its transaction costs no join ([[query#Views and Scans#Virtual Predicates]]).

## Edit Lineage

The full correction history of a fact is a path over `sys:supersedes`, which every [[time-model#Operations#Supersede]] writes from the new root to the old one.

```sparql
SELECT ?old WHERE { v:alice v:age ?a ~ ?r . ?r sys:supersedes+ ?old }
```

A supersede replays the cascade set of the old root, and that set includes the earlier `sys:supersedes` link. So after two corrections, the current root links to both earlier versions, and `sys:supersedes+` returns the whole chain. A skolem IRI (`<urn:tiramemsu:stmt:N>`) of any version starts the same walk.

## Contradictions Between Sources

Two live statements with the same subject and predicate, different objects, overlapping valid time and different authors: a conflict between sources, found on read.

These are the conflicts that `sys:cardinality sys:one` would have prevented on write, for predicates that do not declare it ([[data-model#Predicate Schema]]).

```sparql
SELECT ?s ?o1 ?o2 ?a1 ?a2 WHERE {
  ?s v:worksAt ?o1 ~ ?r1 . ?s v:worksAt ?o2 ~ ?r2 .
  FILTER(STR(?o1) < STR(?o2))
  ?r1 tm:txAdded ?t1 . ?t1 sys:author ?a1 .
  ?r2 tm:txAdded ?t2 . ?t2 sys:author ?a2 .
  FILTER(?a1 != ?a2)
  OPTIONAL { ?r1 tm:validFrom ?f1 } OPTIONAL { ?r1 tm:validTo ?u1 }
  OPTIONAL { ?r2 tm:validFrom ?f2 } OPTIONAL { ?r2 tm:validTo ?u2 }
  FILTER((!BOUND(?f1) || !BOUND(?u2) || ?f1 < ?u2) &&
         (!BOUND(?f2) || !BOUND(?u1) || ?f2 < ?u1)) }
```

The overlap test is the same half-open test that assert uses ([[time-model#Operations#Assert]]); an unbound end is unbounded. Episodes that do not overlap, such as a later job, are not reported. `STR(?o1) < STR(?o2)` reports each pair once.

## When We Learned It

`tm:addedAt` and `tm:retractedAt` give the wall-clock instant a statement was recorded and retracted. Compared with valid time, they measure how late memory learned a fact.

```sparql
# facts recorded after they had already stopped being true
SELECT ?s WHERE { ?s v:worksAt ?o ~ ?r . ?r tm:addedAt ?a ; tm:validTo ?to FILTER(?a > ?to) }
```

```cypher
// learned more than $days days after it became true
MATCH (a)-[r:worksAt]->(c)
WHERE r.addedAt.epochMillis - r.validFrom.epochMillis > $days * 86400000
RETURN a, c
```

Only a store with both clocks on every statement can ask this. The instants are virtual predicates computed from the `tx` table, so they cost one rowid seek and no stored row ([[query#Views and Scans#Virtual Predicates]]). SPARQL 1.1 has no date-time subtraction, so the threshold form is Cypher only. Tests are in `sparql_temporal.rs` and `cypher_temporal.rs`.

## Answers That Cite Their Facts

`View::sparql_with(q, &SparqlOptions { provenance: true })` attaches to every row the eids of the stored statements that produced it. An agent keeps them with its answer and later finds which answers rest on retracted facts.

```rust
let answer = view.sparql_with(q, &SparqlOptions { provenance: true })?;
let cited = answer.solutions().unwrap().provenance(0).unwrap(); // sorted eids
// later: a cited eid that is no longer live marks the answer as stale
```

```sparql
# which cited statements were retracted, and by which transaction
SELECT ?e ?t FROM <urn:tiramemsu:tm:history> WHERE {
  VALUES ?e { <urn:tiramemsu:stmt:12> <urn:tiramemsu:stmt:19> } ?e tm:txRetracted ?t }
```

Provenance counts what matched, including `OPTIONAL` parts, the `UNION` branch taken, annotations and graph memberships, and not what was only tested by `FILTER EXISTS` or `MINUS`. Recursive paths contribute no eids yet. See [[query#Front Ends#SPARQL#Query Provenance]]. Tests are in `sparql_provenance.rs`.

## Typed Layers

`(v:confidence sys:subjectType sys:STMT)` declares that a predicate only annotates statements, so a confidence written on a node by mistake is rejected with `SubjectTypeMismatch` instead of silently becoming a property.

Several values mean any of them, and the flag is checked on every assert and create, from the API, SPARQL and Cypher alike. See [[data-model#Predicate Schema]].

## Portable Facts

A fact travels to another agent's file together with its layers, the statements it rests on and its graph memberships: `View::bundle(eid)` exports it and `Tx::import_bundle` asserts it on the other side ([[data-model#Fact Bundles]]).

```rust
let bundle = agent_a.now().bundle(e1)?.to_json();          // BundleFormat, "tiramemsu-bundle/1"
agent_b.transact(TxOptions::default(), |tx| {
    tx.meta(Value::iri(vocab::SYS_SOURCE), v("agentA"))?;  // provenance of the import
    tx.import_bundle(&Bundle::from_json(&bundle)?)?;
    Ok(())
})?;
```

Import asserts, so importing twice changes nothing, and a fact the target already holds gains the layers on its existing eid. Transaction ids and confirmations stay behind, because they are local to one file; anonymous nodes get fresh ids. `to_ntriples()` writes the same bundle as RDF 1.2 N-Triples with reifiers for other RDF tools.

## Reasoning Inside One Graph

A recursive path under `GRAPH <g>` only crosses statements that are members of `g`, so a session's or an agent's beliefs can be walked without the rest of memory leaking in ([[data-model#Named Graphs]]).

```sparql
SELECT ?x WHERE { GRAPH v:session12 { v:belief9 v:supportedBy/(sys:subject|sys:object)+ ?x } }
SELECT ?g ?x WHERE { GRAPH ?g { v:alice v:knows+ ?x } }   # one row set per graph
```

Membership is read in the path's own view, so under `asOf` the walk follows the graph as it was. `View::path_with` takes `PathArgs { graphs: Some(vec![g]), .. }`, and `tm_path` a sixth `graphs` argument. See [[query#Physical Planning#Path Engine]].

## Metagraphs

A metagraph needs edges that are vertices and vertices that are containers. Statement ids give the first, and graphs as tags give the second, for nodes and for statements ([[data-model#Named Graphs#Statement Graphs]]).

```sparql
# an edge that holds a subgraph, read through its reifier
INSERT DATA { GRAPH <urn:tiramemsu:stmt:1> { v:drSmith v:role v:investigator } }
SELECT ?s WHERE { v:p7 v:enrolledIn ?trial ~ ?e . GRAPH ?e { ?s ?p ?o } }

# metavertices inside metavertices: find them with a path, open them with GRAPH
SELECT ?who WHERE { ?g v:within* v:org . GRAPH ?g { ?who v:owns ?svc } }

# fold is a write of tags (nothing is copied), unfold is a read
INSERT { GRAPH v:episode1 { ?s v:owns ?o } } WHERE { ?s v:owns ?o }
SELECT ?s ?p ?o WHERE { GRAPH v:episode1 { ?s ?p ?o } }
```

N-ary and set-to-set edges are role statements on a node or a statement (`v:alice v:meets v:bob {| v:with v:carol |}`), kept honest by typed layers. Containment and nesting are statements, so `asOf`, `validAt` and time-respecting paths apply to the structure too. In Cypher, `(r)-[:\`sys:inGraph\`]->(g)-[:within*0..]->(top)` reaches the same memberships, one row per path, so a cyclic nesting needs `DISTINCT`. The test is `metagraph_containers_nesting_and_fold`.

## Journeys Through Time

A time-respecting path never goes back in valid time: each fact it crosses must still hold when the walk reaches it. It answers "could this have reached B?", and each row says the earliest instant it could have.

```rust
let args = PathArgs { time_respecting: Some(TimeRespecting { after: None }), ..PathArgs::default() };
let rows = view.path_with(a, "met+", &args)?;   // A met B in [1, 5), B met C in [3, 9)
// B arrives at 1, C at 3; had C met D only in [0, 2), D would not be reached
```

`tm_path` takes the same option as a `timeRespecting` or `timeRespecting/<instant>` part of its view text and returns an `arrival` column. Every mode works, and a property test compares all four with brute-force enumeration of walks. See [[query#Physical Planning#Path Engine#Time-Respecting Search]].
