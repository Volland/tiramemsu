//! Engine IRIs: `sys:`, `tm:`, `xsd:`, `rdf:`, tag IRIs and skolem prefixes.

use crate::id::Tag;

/// `sys:` namespace (engine bookkeeping).
pub const SYS: &str = "urn:tiramemsu:sys:";
/// `tm:` namespace (time IRIs and virtual predicates; reserved for writes).
pub const TM: &str = "urn:tiramemsu:tm:";
/// Default user vocabulary.
pub const V: &str = "urn:tiramemsu:v:";
/// XML Schema datatypes.
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
/// RDF namespace.
pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// Skolem prefix for `NODE` ids.
pub const SKOLEM_NODE: &str = "urn:tiramemsu:node:";
/// Skolem prefix for `BNODE` ids.
pub const SKOLEM_BNODE: &str = "urn:tiramemsu:bnode:";
/// Skolem prefix for statement eids.
pub const SKOLEM_STMT: &str = "urn:tiramemsu:stmt:";
/// Skolem prefix for transactions.
pub const SKOLEM_TX: &str = "urn:tiramemsu:tx:";

/// `xsd:integer`.
pub const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
/// `xsd:boolean`.
pub const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
/// `xsd:string`.
pub const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
/// `xsd:date`.
pub const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";
/// `xsd:dateTime`.
pub const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
/// `xsd:double`.
pub const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
/// `xsd:decimal`.
pub const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
/// `xsd:dayTimeDuration`.
pub const XSD_DAYTIME_DURATION: &str = "http://www.w3.org/2001/XMLSchema#dayTimeDuration";
/// `rdf:langString`.
pub const RDF_LANGSTRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

/// `sys:cardinality`.
pub const SYS_CARDINALITY: &str = "urn:tiramemsu:sys:cardinality";
/// `sys:unique`.
pub const SYS_UNIQUE: &str = "urn:tiramemsu:sys:unique";
/// `sys:valueType`.
pub const SYS_VALUE_TYPE: &str = "urn:tiramemsu:sys:valueType";
/// `sys:isEdge`.
pub const SYS_IS_EDGE: &str = "urn:tiramemsu:sys:isEdge";
/// `sys:subjectType`: the subject kinds a predicate accepts (any-of).
pub const SYS_SUBJECT_TYPE: &str = "urn:tiramemsu:sys:subjectType";
/// `sys:sensitive` (reserved for M6).
pub const SYS_SENSITIVE: &str = "urn:tiramemsu:sys:sensitive";
/// Feature name reported for `sys:sensitive`.
pub const SENSITIVE_FEATURE: &str = "sys:sensitive (M6)";
/// `sys:one`.
pub const SYS_ONE: &str = "urn:tiramemsu:sys:one";
/// `sys:many`.
pub const SYS_MANY: &str = "urn:tiramemsu:sys:many";
/// `sys:confirmedBy` (engine-written).
pub const SYS_CONFIRMED_BY: &str = "urn:tiramemsu:sys:confirmedBy";
/// `sys:supersedes` (engine-written).
pub const SYS_SUPERSEDES: &str = "urn:tiramemsu:sys:supersedes";
/// `sys:author` (transaction metadata).
pub const SYS_AUTHOR: &str = "urn:tiramemsu:sys:author";
/// `sys:source` (transaction metadata).
pub const SYS_SOURCE: &str = "urn:tiramemsu:sys:source";
/// `sys:reason` (transaction metadata).
pub const SYS_REASON: &str = "urn:tiramemsu:sys:reason";
/// `sys:db` (the database node for vocabulary settings).
pub const SYS_DB: &str = "urn:tiramemsu:sys:db";
/// `sys:vocab`.
pub const SYS_VOCAB: &str = "urn:tiramemsu:sys:vocab";

/// `sys:inGraph`: the membership predicate `(eid sys:inGraph graph)` (engine-owned).
pub const SYS_IN_GRAPH: &str = "urn:tiramemsu:sys:inGraph";
/// `sys:derivedFrom`: the support link `(D sys:derivedFrom e)` of a derived belief
/// `D` (engine-owned; written only by `Tx::derive`).
pub const SYS_DERIVED_FROM: &str = "urn:tiramemsu:sys:derivedFrom";
/// `sys:Graph`: the class of a declared named graph (`CREATE GRAPH`).
pub const SYS_GRAPH: &str = "urn:tiramemsu:sys:Graph";
/// Embedding model: the number of dimensions of its vectors (an integer).
pub const SYS_DIMENSIONS: &str = "urn:tiramemsu:sys:dimensions";
/// Embedding model: its similarity metric (`"cosine"`, `"dot"` or `"l2"`).
pub const SYS_METRIC: &str = "urn:tiramemsu:sys:metric";
/// Embedding model: how its vectors are stored (`"f32"` or `"f16"`).
pub const SYS_VECTOR_ENCODING: &str = "urn:tiramemsu:sys:vectorEncoding";
/// `dl:` namespace: stored Datalog programs and the labels of their conclusions.
pub const DL: &str = "urn:tiramemsu:dl:";
/// `dl:source`: `(program dl:source "<text>")`, the one live statement of a
/// stored Datalog program.
pub const DL_SOURCE: &str = "urn:tiramemsu:dl:source";
/// `dl:enforced`: `(program dl:enforced true)` turns a stored program's `error`
/// constraints on at commit.
pub const DL_ENFORCED: &str = "urn:tiramemsu:dl:enforced";
/// `dl:derivedBy`: `(D dl:derivedBy program)`, the program that concluded a
/// materialized statement (a label, not support).
pub const DL_DERIVED_BY: &str = "urn:tiramemsu:dl:derivedBy";
/// `dl:rule`: `(D dl:rule i)`, the index of the rule that concluded it.
pub const DL_RULE: &str = "urn:tiramemsu:dl:rule";
/// `dl:skolem`: the unique key of a node invented by an `exists` head.
pub const DL_SKOLEM: &str = "urn:tiramemsu:dl:skolem";
/// `dl:materialized`: `(tx dl:materialized program)`, a transaction that
/// materialized the program.
pub const DL_MATERIALIZED: &str = "urn:tiramemsu:dl:materialized";
/// `dl:reconciled`: `(tx dl:reconciled program)`, a materialization that
/// retracted conclusions the run no longer derived.
pub const DL_RECONCILED: &str = "urn:tiramemsu:dl:reconciled";
/// `dl:reconciledCount`: `(tx dl:reconciledCount n)`, how many it retracted.
pub const DL_RECONCILED_COUNT: &str = "urn:tiramemsu:dl:reconciledCount";
/// `dl:inf`: the constant a Datalog rule sees for an unbounded valid-time end.
pub const DL_INF: &str = "urn:tiramemsu:dl:inf";
/// `dl:query`: the SPARQL `SERVICE` IRI of a Datalog goal call
/// (`lat.md/query#Datalog Calls`). Not a stored or virtual predicate.
pub const DL_QUERY: &str = "urn:tiramemsu:dl:query";
/// `dl:goal`: in a goal call, the goal body (a string literal).
pub const DL_GOAL: &str = "urn:tiramemsu:dl:goal";
/// `dl:program`: in a goal call, the IRI of a stored program.
pub const DL_PROGRAM: &str = "urn:tiramemsu:dl:program";
/// `dl:text`: in a goal call, the program text (a string literal).
pub const DL_TEXT: &str = "urn:tiramemsu:dl:text";
/// `dl:validFrom`: in a goal call, binds the start of each valid interval.
pub const DL_VALID_FROM: &str = "urn:tiramemsu:dl:validFrom";
/// `dl:validTo`: in a goal call, binds the end of each valid interval.
pub const DL_VALID_TO: &str = "urn:tiramemsu:dl:validTo";
/// `n3:source`: the statement of a stored N3 rule set (the rule-language
/// conflict check reads it without the `n3` feature).
pub const N3_SOURCE: &str = "urn:tiramemsu:n3:source";
/// `tm:Interval`: the datatype of interval values (`"<from>/<to>"` in epoch
/// milliseconds, `..` for an open end).
pub const TM_INTERVAL: &str = "urn:tiramemsu:tm:Interval";
/// `rdf:type`.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The schema-flag predicates.
pub const SCHEMA_FLAGS: [&str; 5] = [
    SYS_CARDINALITY,
    SYS_UNIQUE,
    SYS_VALUE_TYPE,
    SYS_SUBJECT_TYPE,
    SYS_IS_EDGE,
];

/// `sys:` local names users may write as predicates with any subject.
pub const SYS_ALLOWED: [&str; 9] = [
    "cardinality",
    "unique",
    "valueType",
    "subjectType",
    "isEdge",
    "vocab",
    "prefix",
    "prefixName",
    "prefixIri",
];

/// `sys:` local names users may write only with a transaction subject.
pub const SYS_TX_METADATA: [&str; 3] = ["author", "source", "reason"];

/// The tag IRI (`sys:<TAG>`) used as a `sys:valueType` or `sys:subjectType` object.
pub fn tag_iri(tag: Tag) -> String {
    format!("{SYS}{}", tag.name())
}

/// The tag named by a tag IRI, if the IRI is one.
pub fn tag_from_iri(iri: &str) -> Option<Tag> {
    iri.strip_prefix(SYS).and_then(Tag::from_name)
}

/// An IRI in the default user vocabulary.
pub fn v(local: &str) -> String {
    format!("{V}{local}")
}
