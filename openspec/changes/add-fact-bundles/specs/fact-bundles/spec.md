## ADDED Requirements

### Requirement: Bundle members

`View::bundle(root)` SHALL collect the dependents of `root` in the view, plus every statement visible in the view that a collected statement references in subject or object position, transitively. It SHALL NOT add the dependents of a statement that is collected only because it is referenced. When `root` is not visible in the view, `bundle` SHALL fail with `NotLive(root)`.

#### Scenario: Layers, nested layers and a supporting belief
- **WHEN** `e1 = (alice worksAt acme)`, `e2 = (e1 confidence 0.8)`, `e3 = (e2 source "chat")` and `e7 = (belief9 supportedBy e1)` are live, and `bundle(e1)` is read
- **THEN** the bundle holds four statements with the contents of `e1`, `e2`, `e3` and `e7`

#### Scenario: Evidence is carried downward
- **WHEN** `bundle(e7)` is read on the same data
- **THEN** the bundle holds `e7` and `e1`, and not `e2` or `e3`

#### Scenario: Root not visible
- **WHEN** `bundle` is read for a retracted statement on the now view
- **THEN** it fails with `NotLive`

### Requirement: Bundle exclusions

A candidate statement SHALL be excluded from a bundle when its subject or object is a transaction, when its predicate is in the `sys:` or `tm:` namespace and a user write of it would be rejected as reserved (except `sys:inGraph`), or when it references in subject or object position a statement that is not visible in the view or that is excluded. Exclusion SHALL propagate to every statement that references an excluded statement, and the downward closure SHALL be computed from the statements that remain. When `root` itself is excluded, `bundle` SHALL fail with `Unsupported` naming the reason.

#### Scenario: Confirmation is left out
- **WHEN** `e1` has a confirmation `(e1 sys:confirmedBy tx5)` and a layer `(e1 confidence 0.8)`, and `bundle(e1)` is read
- **THEN** the bundle holds `e1` and the confidence, and no statement with `sys:confirmedBy`

#### Scenario: Layers on an excluded statement are left out
- **WHEN** a layer `(c note "seen twice")` stands on the confirmation `c`
- **THEN** that layer is not in the bundle either

#### Scenario: Supersede link is left out
- **WHEN** `e1` was superseded by `e10` and `bundle(e10)` is read on the now view
- **THEN** the bundle holds `e10` and its replayed layers, and no `sys:supersedes` statement

#### Scenario: Transaction metadata root
- **WHEN** `bundle` is read for `(tx5 sys:author agent7)`
- **THEN** it fails with `Unsupported`

### Requirement: Bundle value

A `Bundle` SHALL hold an ordered list of statements, each with a bundle-local id, a subject, a predicate IRI, an object and a valid-time interval, and the local id of the root. A subject or object SHALL be a value (an IRI or a literal), a bundle-local statement id, or a bundle-local anonymous node label. Local ids SHALL be the positions of the statements. Every statement SHALL come after the statements it references when the references are acyclic; ties SHALL be broken by source eid, and statements on a reference cycle SHALL follow the others in eid order. The same view of the same data SHALL always produce an equal bundle.

#### Scenario: References come first
- **WHEN** `bundle(e7)` is read for `e7 = (belief9 supportedBy e1)`
- **THEN** the statement of `e1` comes before the statement of `e7`, and the object of `e7` is the local id of `e1`

#### Scenario: Stable output
- **WHEN** the same bundle is read twice
- **THEN** the two bundles are equal and serialize to the same JSON text

### Requirement: Anonymous nodes

`NODE` and `BNODE` ids SHALL be exported as bundle-local anonymous labels, numbered from 0 in order of first appearance, and never as skolem IRIs. Import SHALL mint one fresh `NODE` per label, so distinct anonymous nodes stay distinct and one node used by several statements stays one node. Import SHALL reject a value that is a skolem IRI of a statement, node, blank node or transaction with `InvalidTerm`.

#### Scenario: Two anonymous nodes stay distinct and consistent
- **WHEN** a bundle mentions anonymous node `n1` in two statements and anonymous node `n2` in one, and is imported
- **THEN** the two statements of `n1` share one new node, and `n2` becomes another new node

#### Scenario: Skolem IRI rejected
- **WHEN** a bundle whose object is the value `urn:tiramemsu:stmt:3` is imported
- **THEN** the import fails with `InvalidTerm` and nothing is written

### Requirement: Import is idempotent assert

`Tx::import_bundle(bundle)` SHALL write every statement in bundle order within the caller's transaction: a `sys:inGraph` statement through `add_to_graph`, every other statement through `assert` with the statement's valid time. Schema and reserved-namespace checks SHALL apply as for any user write. It SHALL return the mapping from every local id to its eid with a `new` flag, in bundle order, and the eid of the root. Importing a bundle without anonymous nodes a second time SHALL change nothing. Importing onto a database that already holds a live statement with the root's content and an overlapping valid time SHALL map the root to that eid and attach the layers to it.

#### Scenario: Round trip between two databases
- **WHEN** a bundle with layers, a nested layer, a belief that references the root, a graph membership and bounded valid times is read from database A and imported into an empty database B
- **THEN** B holds statements with the same contents, the same layer structure, the same membership and the same valid times, and every mapping entry is new

#### Scenario: Second import changes nothing
- **WHEN** the same bundle is imported into B again
- **THEN** every mapping entry has `new` false, the transaction asserts nothing, and B's statements are unchanged

#### Scenario: Existing root fact
- **WHEN** B already holds `(alice worksAt acme)` and a bundle rooted on that fact with a confidence layer is imported
- **THEN** the root maps to B's existing eid with `new` false, and the confidence is asserted on that eid

#### Scenario: Schema violation fails atomically
- **WHEN** `email` is `sys:unique` in B and `(bob email "x")` is live, and a bundle holding `(alice email "x")` is imported
- **THEN** the transaction fails with `UniqueViolation` and nothing from the bundle is stored

### Requirement: Reference cycles are not imported

`import_bundle` SHALL fail with `Unsupported { feature: "bundle with a reference cycle" }` before writing anything when the statements of the bundle reference each other in a cycle. A malformed bundle (a duplicate or unknown local id, a reference to a missing statement, a predicate that is not an IRI, an unknown root) SHALL fail with `InvalidTerm` before writing anything.

#### Scenario: Cycle
- **WHEN** a bundle is read for a statement on a reference cycle and imported
- **THEN** the import fails with `Unsupported` and the target is unchanged

### Requirement: Bundle of the past

`bundle` SHALL use the view for every statement it reads, so an as-of view SHALL produce the bundle of a structure as it was, including statements retracted since.

#### Scenario: Since-retracted structure
- **WHEN** `e1` with its layers was retracted in transaction 4 and `bundle(e1)` is read as of transaction 3 and imported into an empty database
- **THEN** the target holds live copies of `e1` and its layers

### Requirement: Bundle JSON

`BundleFormat::to_json` SHALL produce `{"format": "tiramemsu-bundle/1", "root", "statements"}` where every statement is `{"id", "s", "p", "o"}` plus `validFrom` and `validTo` in epoch milliseconds when bounded, `p` is the predicate IRI text, and a term is `{"iri"}`, `{"ref": id}`, `{"blank": label}`, `{"lex", "datatype"}` or `{"lex", "lang"}`. `BundleFormat::from_json` SHALL accept exactly that form and SHALL reject another format string or a malformed statement with `InvalidTerm`. `from_json(to_json(b))` SHALL equal `b`.

#### Scenario: JSON round trip is stable
- **WHEN** a bundle holding an IRI, a string, a language string, an integer, a double, a date-time with an offset, an anonymous node and a statement reference is written to JSON and read back
- **THEN** the result equals the original bundle and writes the same JSON again

#### Scenario: Unknown version
- **WHEN** `from_json` reads `{"format": "tiramemsu-bundle/2", ...}`
- **THEN** it fails with `InvalidTerm`

### Requirement: Bundle N-Triples

`BundleFormat::to_ntriples` SHALL produce RDF 1.2 N-Triples: for every statement its triple, the reifier triple `_:s<i> rdf:reifies <<( s p o )>>`, and `tm:validFrom` / `tm:validTo` annotations on the reifier for a bounded valid time. A statement in subject or object position SHALL be written as its reifier blank node and an anonymous node as a blank node. The output SHALL parse as RDF 1.2 N-Triples.

#### Scenario: Layer as an annotation
- **WHEN** a bundle of `e1 = (alice worksAt acme)` and `(e1 confidence 0.8)` is written as N-Triples
- **THEN** the text holds the triple of `e1`, a `rdf:reifies` triple whose object is the triple term of `e1`, and a triple whose subject is the reifier of `e1` and whose predicate is `confidence`, and it parses as RDF 1.2 N-Triples

### Requirement: Bundles in the JSON bridge

The JSON bridge SHALL offer the read `bundle` (`eid`, `view`) returning the bundle JSON, and the transaction op `importBundle` (`bundle`) returning `{"root", "statements": [{"id", "eid", "new"}]}`. An `importBundle` op MAY carry `as`, which names the imported root.

#### Scenario: Bridge round trip
- **WHEN** `bundle` is read from one bridge database and passed to `importBundle` in a `transact` call on another
- **THEN** the second database holds the statements, and the result maps every id to a new eid
