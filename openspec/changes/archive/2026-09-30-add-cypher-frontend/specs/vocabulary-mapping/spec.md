## Purpose

Defines how Cypher names (labels, relationship types, property keys and node identities) map to and from RDF IRIs, through one configurable `@vocab` base and a versioned prefix table. The same data is then addressable identically from Cypher and SPARQL, and engine-internal `sys:` data stays hidden from ordinary Cypher views.

## ADDED Requirements

### Requirement: Bare names resolve under @vocab
A label, relationship type or property key written as a bare identifier, or as a backticked name without a `:`, SHALL resolve to the IRI formed by the current `@vocab` base followed by the name verbatim, with no case change. Characters that are not allowed in an IRI SHALL be percent-encoded as UTF-8. The default `@vocab` SHALL be `urn:tiramemsu:v:`.

#### Scenario: Case is preserved
- **WHEN** `(v:alice rdf:type <urn:tiramemsu:v:Person>)` is live and `MATCH (n:person) RETURN n` runs
- **THEN** zero rows are returned
- **AND** `MATCH (n:Person) RETURN n` returns `v:alice`

#### Scenario: Relationship type and key
- **WHEN** `MATCH (a)-[r:WORKS_AT {since: 2020}]->(c) RETURN a` is compiled
- **THEN** the pattern uses predicate `urn:tiramemsu:v:WORKS_AT` and property predicate `urn:tiramemsu:v:since`

#### Scenario: Backticked name with a space
- **WHEN** ``CREATE (n:`Top Customer`)`` runs through the write entry point
- **THEN** the asserted label object is `urn:tiramemsu:v:Top%20Customer`
- **AND** `labels(n)` returns `["Top Customer"]`

### Requirement: Backticked CURIEs resolve through the prefix table
A backticked name of the form `prefix:local`, whose prefix is declared, SHALL resolve to the prefix's IRI followed by `local`. The built-in prefixes SHALL always be declared: `sys` → `urn:tiramemsu:sys:`, `tm` → `urn:tiramemsu:tm:`, `rdf` → `http://www.w3.org/1999/02/22-rdf-syntax-ns#`, `rdfs` → `http://www.w3.org/2000/01/rdf-schema#`, `xsd` → `http://www.w3.org/2001/XMLSchema#`, and `v` → the current `@vocab`. User prefixes SHALL come from the prefix table.

#### Scenario: User prefix
- **WHEN** prefix `schema` → `https://schema.org/` is declared, `(v:alice <https://schema.org/name> "Alice")` is live, and ``MATCH (n) WHERE n.`schema:name` = 'Alice' RETURN n`` runs
- **THEN** `v:alice` is returned

#### Scenario: Built-in rdfs prefix
- **WHEN** ``MATCH (n) RETURN n.`rdfs:label` `` is compiled
- **THEN** the key resolves to `http://www.w3.org/2000/01/rdf-schema#label`

#### Scenario: v prefix equals @vocab
- **WHEN** ``MATCH (n:`v:Person`) RETURN n`` and `MATCH (n:Person) RETURN n` run
- **THEN** both return the same rows

### Requirement: Backticked full IRIs resolve verbatim
A backticked name containing `:` whose part before the first `:` is not a declared prefix SHALL be taken as an absolute IRI, verbatim. A name that is neither a declared CURIE nor a syntactically valid absolute IRI MUST fail with a `Parse` error spanning it.

#### Scenario: Full IRI label
- **WHEN** `(v:acme rdf:type <https://schema.org/Organization>)` is live, no prefix maps `https://schema.org/`, and ``MATCH (n:`https://schema.org/Organization`) RETURN n`` runs
- **THEN** `v:acme` is returned

#### Scenario: Invalid IRI
- **WHEN** ``MATCH (n:`1bad:<>`) RETURN n`` is compiled
- **THEN** it fails with a `Parse` error

### Requirement: Unknown names match nothing
A name that resolves to an IRI never used in the store SHALL make its pattern match nothing, and a property access with it SHALL yield `null`. Neither SHALL be an error.

#### Scenario: Unknown label
- **WHEN** `MATCH (n:NeverUsed) RETURN count(n) AS c` runs
- **THEN** `c` is 0

### Requirement: IRIs render as Cypher names
Wherever the engine returns a name (`labels()`, `type()`, `keys()`, the keys of `properties()`, map keys, node labels and relationship types in results, and built-in procedure output), it SHALL render the IRI as follows:
1. If the IRI starts with the current `@vocab` and the remaining local name contains no `:`, it SHALL render as that local name, percent-decoded.
2. Otherwise, if a declared prefix's IRI is a prefix of it, it SHALL render as the CURIE `prefix:local`, using the longest matching prefix IRI, with ties broken by the lexicographically smallest prefix name.
3. Otherwise it SHALL render as the full IRI.

Rendered names in result values SHALL NOT include backticks. Backticks are only query-text quoting.

#### Scenario: Local name
- **WHEN** `labels(n)` runs for a node typed `urn:tiramemsu:v:Person`
- **THEN** it returns `["Person"]`

#### Scenario: CURIE and full IRI
- **WHEN** a node has types `https://schema.org/Person` (with prefix `schema` declared) and `http://example.org/X` (no prefix), and `labels(n)` runs
- **THEN** it returns `["http://example.org/X", "schema:Person"]`, in ascending order

#### Scenario: Local name containing a colon is not shortened
- **WHEN** a key IRI is `urn:tiramemsu:v:schema:name` and `keys(n)` runs
- **THEN** the key renders as `"v:schema:name"`, never as `"schema:name"`

### Requirement: Names round-trip
Every name the engine renders SHALL resolve back to the same IRI when written as a backticked name in a later query under the same vocabulary configuration.

#### Scenario: Round trip of rendered labels
- **WHEN** a query returns a label name `L` from `labels(n)` and a second query ``MATCH (m:`L`) WHERE m = $n RETURN m`` runs, with `L` substituted verbatim inside the backticks
- **THEN** the node is returned, for local names, CURIEs and full IRIs alike

### Requirement: Labels are rdf:type statements
A Cypher label SHALL be the object of an `rdf:type` statement. `labels(n)` SHALL return the rendered names of the objects of the visible `rdf:type` statements of `n`, without duplicates, in ascending order. For statement nodes, the implicit `Statement` label comes first.

#### Scenario: Labels from SPARQL-written types
- **WHEN** a SPARQL update inserts `v:alice a v:Person, v:Agent` and ``MATCH (n {`@id`: 'v:alice'}) RETURN labels(n) AS l`` runs
- **THEN** `l` is `["Agent", "Person"]`

### Requirement: Reserved implicit labels
The label names `Statement` and `Predicate` SHALL be implicit. `:Statement` SHALL match statements (see `cypher-dual-view`). `:Predicate` SHALL match predicate IRIs that are the subject of a visible predicate-schema flag (`sys:cardinality`, `sys:unique`, `sys:valueType` or `sys:isEdge`). On `:Predicate` nodes those flags SHALL be readable through their CURIEs; IRI-valued flags SHALL render as names. A user class whose IRI is `@vocab` followed by `Statement` or `Predicate` SHALL be reachable only through its CURIE or full IRI.

#### Scenario: Query the predicate schema
- **WHEN** `(v:email sys:unique true)` and `(v:age sys:cardinality sys:one)` are live and ``MATCH (p:Predicate) RETURN p, p.`sys:unique` AS u, p.`sys:cardinality` AS card ORDER BY elementId(p)`` runs
- **THEN** the rows are `(v:age, null, "sys:one")` and `(v:email, true, null)`

#### Scenario: User class named Statement
- **WHEN** `(v:x rdf:type <urn:tiramemsu:v:Statement>)` is live and ``MATCH (n:`v:Statement`) RETURN n`` runs
- **THEN** `v:x` is returned, while `MATCH (n:Statement)` does not return `v:x` unless it is a statement

### Requirement: sys namespace is hidden by default
Statements whose predicate or `rdf:type` object is in the `sys:` namespace SHALL be excluded from `labels()`, `keys()`, `properties()`, node and relationship values in results, untyped relationship patterns, the unlabelled node scan, `:Statement` enumeration and built-in name procedures. Explicit access by CURIE or full IRI (``n.`sys:reason` ``, ``-[:`sys:supersedes`]->``) SHALL still match them.

#### Scenario: Hidden from keys
- **WHEN** `(v:email sys:unique true)` and `(v:email rdfs:label "e-mail")` are live and ``MATCH (p {`@id`: 'v:email'}) RETURN keys(p) AS k`` runs
- **THEN** `k` is `["rdfs:label"]`

#### Scenario: Explicit access to a supersede chain
- **WHEN** e10 superseded e1 and ``MATCH (new)-[:`sys:supersedes`]->(old) RETURN elementId(new), elementId(old)`` runs
- **THEN** one row with the element ids of e10 and e1 is returned

#### Scenario: Transaction metadata hidden
- **WHEN** `(tx5 sys:author v:agent7)` is live and `v:agent7` is mentioned by no other statement, and `MATCH (n) RETURN n` runs
- **THEN** `v:agent7` is not returned

### Requirement: Node identity and @id
`elementId(x)` SHALL return an IRI string: the IRI itself for IRI nodes, `urn:tiramemsu:node:<n>` for anonymous nodes, `urn:tiramemsu:bnode:<n>` for blank nodes, and `urn:tiramemsu:stmt:<n>` for statements. `id(x)` SHALL return the 64-bit storage identifier as an Integer. The reserved key `` `@id` `` in a node pattern's property map SHALL constrain (in `MATCH`) or choose (in `CREATE` and `MERGE`) the node's identity. Its value SHALL be a string holding a declared CURIE, an absolute IRI, or one of the skolem forms above. A skolem IRI SHALL resolve to the same anonymous node, blank node or statement it was rendered from. `@id` SHALL NOT be stored as a property, nor listed by `keys()`.

#### Scenario: Match by IRI
- **WHEN** ``MATCH (n {`@id`: 'v:alice'}) RETURN elementId(n) AS id`` runs
- **THEN** `id` is `"urn:tiramemsu:v:alice"`

#### Scenario: Skolem round trip for anonymous nodes
- **WHEN** `CREATE (n:Person {name:'Bob'}) RETURN elementId(n) AS id` returns `urn:tiramemsu:node:12`, and ``MATCH (m {`@id`: 'urn:tiramemsu:node:12'}) RETURN m.name`` runs
- **THEN** the value is `"Bob"`

#### Scenario: Match by element id in WHERE
- **WHEN** `MATCH (n) WHERE elementId(n) = $id RETURN n` runs with `id` set to a previously returned element id
- **THEN** exactly that node is returned

#### Scenario: Invalid @id
- **WHEN** ``MATCH (n {`@id`: 'not an iri'}) RETURN n`` runs
- **THEN** it fails with an `Eval` error

### Requirement: Vocabulary configuration
The `@vocab` base SHALL be stored as the statement `(sys:db sys:vocab <iri>)`. Each prefix SHALL be stored as `(sys:db sys:prefix _:p)` with `(_:p sys:prefixName "name")` and `(_:p sys:prefixIri <iri>)`. Both SHALL be changed only through transactions, so they are versioned. Setting the vocab SHALL replace the previous live vocab statement. Declaring an existing prefix name SHALL replace its IRI. Prefix names MUST match `[A-Za-z][A-Za-z0-9_-]*`, and redeclaring a built-in prefix (`sys`, `tm`, `rdf`, `rdfs`, `xsd`, `v`) MUST fail with `ReservedNamespace`. Name resolution and rendering SHALL always use the vocabulary configuration that is current when the query is compiled, even for queries with time clauses. Changing it SHALL NOT rewrite stored data.

#### Scenario: Change @vocab
- **WHEN** labels were written under the default vocab, the vocab is then set to `https://ex.org/`, and `MATCH (n) RETURN labels(n)` runs
- **THEN** the old labels render as full IRIs such as `"urn:tiramemsu:v:Person"`, because no declared prefix covers the old base
- **AND** `CREATE (m:Person)` now asserts type `https://ex.org/Person`

#### Scenario: Historical query uses current vocabulary
- **WHEN** prefix `schema` was declared in tx 9, and ``USE AS OF 5 MATCH (n) WHERE n.`schema:name` IS NOT NULL RETURN n`` runs
- **THEN** `schema:` resolves through the current prefix table, and the query returns the nodes that had a `https://schema.org/name` property as of tx 5

#### Scenario: Reserved prefix name
- **WHEN** a transaction declares prefix `rdf` → `https://evil.example/`
- **THEN** it fails with `ReservedNamespace`, and the transaction leaves no trace

### Requirement: Shared addressing with SPARQL
A bare Cypher name `N` and the SPARQL IRI `@vocab` + `N` SHALL address the same predicate or class, and data written through either dialect SHALL be visible through the other under the mapping above.

#### Scenario: Cypher sees SPARQL data
- **WHEN** SPARQL `INSERT DATA { v:alice v:worksAt v:acme }` runs (with `v:` bound to the vocab) and then `MATCH (a)-[:worksAt]->(c) RETURN elementId(a), elementId(c)` runs
- **THEN** the row is `("urn:tiramemsu:v:alice", "urn:tiramemsu:v:acme")`
