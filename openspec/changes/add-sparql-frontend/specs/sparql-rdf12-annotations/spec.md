## Purpose

Exposes Tiramemsu's addressable statements through SPARQL 1.2 syntax. Reifiers and triple terms bind directly to statement eids, and annotations read and write layer triples whose subject is an eid, nested to any depth. Provenance, confidence and beliefs about facts are then plain SPARQL.

## ADDED Requirements

### Requirement: Reifiers bind statement eids

In a query pattern, a reifier on a triple (`s p o ~ ?r`), a reified triple (`<< s p o ~ ?r >>`) and the explicit form `?r rdf:reifies <<( s p o )>>` SHALL each bind `?r` to the eid of a statement with content `(s, p, o)` that is visible in the pattern's view. Each such eid SHALL produce its own solution. A reifier written as a blank node, or left out as in `<< s p o >>`, SHALL behave as an unprojected variable. A reified triple SHALL match only statements that exist in the view, because every eid in the store is a stored statement.

#### Scenario: Reifier binds the eid
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` is live and `SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }` is run
- **THEN** one row is returned whose `r` is the statement IRI of e1

#### Scenario: Explicit rdf:reifies form is equivalent
- **WHEN** the same data is queried with `SELECT ?r WHERE { ?r rdf:reifies <<( v:alice v:worksAt v:acme )>> }`
- **THEN** the same single row is returned

#### Scenario: One row per episode
- **WHEN** `(v:alice v:worksAt v:acme)` is live as two episodes e1 and e2 and `SELECT ?r WHERE { v:alice v:worksAt ?c ~ ?r }` is run
- **THEN** two rows are returned, for e1 and e2

#### Scenario: Reified triple as subject
- **WHEN** e1 carries `(e1 v:confidence 0.8)` and `SELECT ?c WHERE { << v:alice v:worksAt v:acme >> v:confidence ?c }` is run
- **THEN** one row with `c = 0.8` (an `xsd:decimal`) is returned

#### Scenario: Reifier constant that is not a statement
- **WHEN** `ASK { v:alice v:worksAt v:acme ~ v:someIri }` is run
- **THEN** the answer is `false` and no error is raised

### Requirement: Annotation syntax matches layer triples

An annotation block `s p o ~ ?r {| q1 v1 ; q2 v2 |}` (with or without an explicit reifier) SHALL match only when the statement `(s, p, o)` is visible and every annotation triple `(eid q1 v1)`, `(eid q2 v2)` is visible, all in the same view scope. Annotation triples SHALL be required, not optional. A missing annotation SHALL remove the solution unless the query wraps it in `OPTIONAL`.

#### Scenario: Annotation value is read
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` has `(e1 v:confidence 0.8)` and `(e1 v:source v:crawler)`, and `SELECT ?c ?s WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ; v:source ?s |} }` is run
- **THEN** one row with `c = 0.8`, `s = v:crawler` is returned

#### Scenario: Missing annotation removes the solution
- **WHEN** `(v:bob v:worksAt v:acme)` is live without any annotation and `SELECT ?p WHERE { ?p v:worksAt v:acme {| v:confidence ?c |} }` is run
- **THEN** `v:bob` is not returned

#### Scenario: Optional annotation
- **WHEN** `SELECT ?p ?c WHERE { ?p v:worksAt v:acme ~ ?r OPTIONAL { ?r v:confidence ?c } }` is run over the same data
- **THEN** `v:bob` is returned with `c` unbound

### Requirement: Nested layers

Reifiers and annotations SHALL nest to any depth. An annotation triple may itself carry a reifier and an annotation block. A reified triple or triple term may occur inside another triple term. An eid bound by a reifier SHALL be usable in the subject or object position of any other triple pattern, so references to statements (for example beliefs that point at facts) can be matched.

#### Scenario: Annotation on an annotation
- **WHEN** e1 = `(v:alice v:worksAt v:acme)`, e2 = `(e1 v:confidence 0.8)` and e3 = `(e2 v:method "llm-extraction")` are live, and `SELECT ?c ?m WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ~ ?r2 {| v:method ?m |} |} }` is run
- **THEN** one row with `c = 0.8`, `m = "llm-extraction"` is returned

#### Scenario: Belief referencing a fact
- **WHEN** e7 = `(v:belief9 v:supportedBy e1)` is live and `SELECT ?b WHERE { ?b v:supportedBy ?r . ?r rdf:reifies <<( v:alice v:worksAt ?c )>> }` is run
- **THEN** one row with `b = v:belief9` is returned

#### Scenario: Nested triple term
- **WHEN** e5 = `(v:bob v:says e1)` and e6 = `(v:carol v:doubts e5)` are live and `SELECT ?who WHERE { ?who v:doubts <<( v:bob v:says <<( v:alice v:worksAt v:acme )>> )>> }` is run
- **THEN** one row with `who = v:carol` is returned

### Requirement: Triple terms denote statements

A triple term `<<( s p o )>>` in the object of a pattern whose predicate is not `rdf:reifies` SHALL match an object that is the eid of a visible statement with content `(s, p, o)`. Because the graph is a set of triples, a reference SHALL be matched once per distinct referring triple, not once per matching eid, unless an eid is bound.

#### Scenario: Triple term in object position
- **WHEN** `(v:belief9 v:supportedBy e1)` is live with e1 = `(v:alice v:worksAt v:acme)`, and `ASK { v:belief9 v:supportedBy <<( v:alice v:worksAt v:acme )>> }` is run
- **THEN** the answer is `true`

### Requirement: rdf:reifies is virtual

`rdf:reifies` SHALL NOT be stored. A pattern with a variable predicate SHALL NOT return `rdf:reifies` rows. A pattern `?r rdf:reifies ?t` whose object is not a triple term SHALL fail with `Unsupported { feature: "rdf:reifies without triple term" }`. A pattern with a variable predicate and a triple-term object SHALL fail with `Unsupported { feature: "variable predicate with triple term" }`. The SPARQL 1.2 triple functions `TRIPLE`, `SUBJECT`, `PREDICATE`, `OBJECT` and `isTRIPLE` SHALL fail with `Unsupported` naming the function.

#### Scenario: Variable predicate does not see rdf:reifies
- **WHEN** only e1 = `(v:alice v:worksAt v:acme)` is live and `SELECT ?p WHERE { ?s ?p ?o }` is run
- **THEN** exactly one row with `p = v:worksAt` is returned

#### Scenario: rdf:reifies with variable object
- **WHEN** `SELECT ?t WHERE { ?r rdf:reifies ?t }` is submitted
- **THEN** the request fails with `Unsupported { feature: "rdf:reifies without triple term" }`

#### Scenario: Triple function unsupported
- **WHEN** `SELECT ?x WHERE { ?x v:about ?o FILTER(isTRIPLE(?o)) }` is submitted
- **THEN** the request fails with `Unsupported { feature: "isTRIPLE" }`

### Requirement: Reifiers follow the pattern's view

A reifier SHALL bind eids visible in the view of the pattern it belongs to. Under the history view, retracted eids SHALL also bind. Under an as-of view, only the eids live at that transaction SHALL bind. Annotation triples SHALL be read in the same view as their base triple unless a time `SERVICE` scope says otherwise.

#### Scenario: History shows a superseded fact and its replacement
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` was superseded by e10 with a new valid start, and `SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }` is run with `FROM <urn:tiramemsu:tm:history>`
- **THEN** two rows are returned, for e1 and e10, while the same query on the current view returns only e10

#### Scenario: Cascaded annotation visible as of before retraction
- **WHEN** e1 and its annotation e2 were retracted in tx 9, and `SELECT ?c WHERE { v:alice v:worksAt v:acme {| v:confidence ?c |} }` is run on the view as of tx 8
- **THEN** one row with `c = 0.8` is returned

### Requirement: Same eid across dialects and the API

The eid bound by a SPARQL reifier SHALL be the same statement identity returned by the Rust API for that statement and bound by a Cypher relationship variable for the same fact.

#### Scenario: API eid equals SPARQL reifier
- **WHEN** the API asserts `(v:alice v:worksAt v:acme)` and gets eid e, and `SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }` is run
- **THEN** `?r` is the statement IRI of e, and the query `SELECT ?c WHERE { <urn:tiramemsu:stmt:N> v:confidence ?c }`, with N the number of e, reads annotations of that same statement

### Requirement: Inserting annotations creates layer triples on the eid

In `INSERT DATA` and insert templates, a triple with a reifier or an annotation block (`s p o {| q v |}` or `s p o ~ _:r {| q v |}`) SHALL assert `(s, p, o)` idempotently. The reifier SHALL then denote that statement's eid, new or existing, and each annotation triple SHALL be asserted idempotently with that eid as subject. A reified triple or triple term used as a subject or object in inserted data (for example `v:belief9 v:supportedBy << v:alice v:worksAt v:acme >>`) SHALL assert `(s, p, o)` idempotently and use its eid in that position. This is a deliberate deviation from RDF 1.2, where a reified triple is not asserted: in this store every reifier is a stored statement. A variable reifier bound by `WHERE` to an eid SHALL let the template attach triples to that eid.

#### Scenario: INSERT DATA with annotation
- **WHEN** `INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.8 ; v:source v:crawler |} }` is submitted on an empty store
- **THEN** three statements are live: e1 = `(v:alice v:worksAt v:acme)`, `(e1 v:confidence 0.8)` and `(e1 v:source v:crawler)`, and no `rdf:reifies` triple is stored

#### Scenario: Annotation insert is idempotent
- **WHEN** the same request is submitted again
- **THEN** no new statement is created and the report lists all three eids as existing

#### Scenario: Annotating an existing statement
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` is live and `INSERT DATA { v:alice v:worksAt v:acme {| v:confidence 0.9 |} }` is submitted
- **THEN** e1 is reported as existing, and `(e1 v:confidence 0.9)` is asserted with e1 as subject

#### Scenario: Reference to a reified triple asserts it
- **WHEN** `INSERT DATA { v:belief9 v:supportedBy << v:alice v:worksAt v:acme >> }` is submitted on an empty store
- **THEN** e1 = `(v:alice v:worksAt v:acme)` and `(v:belief9 v:supportedBy e1)` are both live

#### Scenario: Template attaches to a bound eid
- **WHEN** e1 and e2 are two live episodes of `(v:alice v:worksAt v:acme)` and `INSERT { ?r v:reviewed true } WHERE { v:alice v:worksAt v:acme ~ ?r }` is submitted
- **THEN** `(e1 v:reviewed true)` and `(e2 v:reviewed true)` are both asserted

### Requirement: Reifier restrictions in updates

In an update, a reifier SHALL be a blank node, a variable bound to a statement eid, or the statement IRI of a stored statement whose content equals the reified triple. Any other reifier (an ordinary IRI, a literal, or a statement IRI whose content differs) SHALL fail the request with `Unsupported { feature: "reifier that is not a statement" }`. A reifier that would reify two different triples in one request SHALL fail with `Unsupported { feature: "reifier of more than one triple" }`. Nothing SHALL be written in either case.

#### Scenario: IRI reifier rejected
- **WHEN** `INSERT DATA { v:alice v:worksAt v:acme ~ v:myReifier {| v:confidence 0.8 |} }` is submitted
- **THEN** the request fails with `Unsupported { feature: "reifier that is not a statement" }` and nothing is written

#### Scenario: Statement IRI reifier accepted
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` is live with number 12 and `INSERT DATA { v:alice v:worksAt v:acme ~ <urn:tiramemsu:stmt:12> {| v:confidence 0.8 |} }` is submitted
- **THEN** `(e1 v:confidence 0.8)` is asserted and e1 is reported as existing

#### Scenario: One reifier for two triples rejected
- **WHEN** `INSERT DATA { _:r rdf:reifies <<( v:a v:b v:c )>> . _:r rdf:reifies <<( v:d v:e v:f )>> }` is submitted
- **THEN** the request fails with `Unsupported { feature: "reifier of more than one triple" }`

### Requirement: Deleting through reifiers is eid-precise

In a delete template, a triple that carries a reifier bound to an eid SHALL retract exactly that eid, with cascade. It SHALL NOT retract other eids with the same content. A delete template triple whose subject is a bound eid SHALL retract the matching layer triples on that eid and leave the eid itself live. Delete templates SHALL name reifiers with variables, because blank nodes are not allowed in delete templates.

#### Scenario: Retract one episode only
- **WHEN** e1 and e2 are live episodes of `(v:alice v:worksAt v:acme)`, only e1 has `(e1 v:source v:crawler)`, and `DELETE { ?s v:worksAt ?o ~ ?r } WHERE { ?s v:worksAt ?o ~ ?r {| v:source v:crawler |} }` is submitted
- **THEN** e1 and its annotation are retracted (kinds explicit and cascade) and e2 stays live

#### Scenario: Retract an annotation only
- **WHEN** e1 has `(e1 v:confidence 0.8)` and `DELETE { ?r v:confidence ?c } WHERE { v:alice v:worksAt v:acme ~ ?r {| v:confidence ?c |} }` is submitted
- **THEN** the confidence statement is retracted and e1 stays live

### Requirement: CONSTRUCT emits RDF 1.2 reification

A `CONSTRUCT` template with a reifier or an annotation SHALL produce, for each solution, the triple `(s, p, o)`, the triple `(reifier rdf:reifies <<( s p o )>>)`, and one triple per annotation with the reifier as subject. A reifier bound to an eid SHALL be rendered as that statement's IRI. An eid in any other position SHALL also be rendered as its statement IRI.

#### Scenario: Export an annotated fact
- **WHEN** e1 (number 12) = `(v:alice v:worksAt v:acme)` has `(e1 v:confidence 0.8)`, and `CONSTRUCT { ?s v:worksAt ?o ~ ?r {| v:confidence ?c |} } WHERE { ?s v:worksAt ?o ~ ?r {| v:confidence ?c |} }` is run
- **THEN** the result is exactly the three triples `v:alice v:worksAt v:acme`, `<urn:tiramemsu:stmt:12> rdf:reifies <<( v:alice v:worksAt v:acme )>>` and `<urn:tiramemsu:stmt:12> v:confidence 0.8`, and its N-Triples serialisation uses the RDF 1.2 triple-term syntax
