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

/// The four schema-flag predicates.
pub const SCHEMA_FLAGS: [&str; 4] = [SYS_CARDINALITY, SYS_UNIQUE, SYS_VALUE_TYPE, SYS_IS_EDGE];

/// `sys:` local names users may write as predicates with any subject.
pub const SYS_ALLOWED: [&str; 8] = [
    "cardinality",
    "unique",
    "valueType",
    "isEdge",
    "vocab",
    "prefix",
    "prefixName",
    "prefixIri",
];

/// `sys:` local names users may write only with a transaction subject.
pub const SYS_TX_METADATA: [&str; 3] = ["author", "source", "reason"];

/// The tag IRI (`sys:<TAG>`) used as a `sys:valueType` object.
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
