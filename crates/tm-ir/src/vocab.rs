//! IRIs of the virtual predicates (`lat.md/query#Views and Scans#Virtual Predicates`).

use tm_core::vocab::{SYS, TM};

/// `sys:inGraph`: the membership predicate of a named graph.
pub const SYS_IN_GRAPH: &str = "urn:tiramemsu:sys:inGraph";
/// `sys:subject`: the subject of the statement named by the subject.
pub const SYS_SUBJECT: &str = "urn:tiramemsu:sys:subject";
/// `sys:predicate`: the predicate of the statement.
pub const SYS_PREDICATE: &str = "urn:tiramemsu:sys:predicate";
/// `sys:object`: the object of the statement.
pub const SYS_OBJECT: &str = "urn:tiramemsu:sys:object";
/// `sys:anyRelationship`: the Cypher `[*]` wildcard in path text.
pub const SYS_ANY_RELATIONSHIP: &str = "urn:tiramemsu:sys:anyRelationship";
/// `tm:txAdded`: the transaction that added the statement.
pub const TM_TX_ADDED: &str = "urn:tiramemsu:tm:txAdded";
/// `tm:txRetracted`: the transaction that retracted the statement.
pub const TM_TX_RETRACTED: &str = "urn:tiramemsu:tm:txRetracted";
/// `tm:addedAt`: the commit instant of the transaction that added the statement.
pub const TM_ADDED_AT: &str = "urn:tiramemsu:tm:addedAt";
/// `tm:retractedAt`: the commit instant of the transaction that retracted it.
pub const TM_RETRACTED_AT: &str = "urn:tiramemsu:tm:retractedAt";
/// `tm:validFrom`: the start of the statement's valid interval.
pub const TM_VALID_FROM: &str = "urn:tiramemsu:tm:validFrom";
/// `tm:validTo`: the end of the statement's valid interval.
pub const TM_VALID_TO: &str = "urn:tiramemsu:tm:validTo";
/// `tm:retractKind`: 0 explicit, 1 cascade, 2 supersede, 3 cardinality.
pub const TM_RETRACT_KIND: &str = "urn:tiramemsu:tm:retractKind";

/// `tm:textMatch`: in SPARQL, `?e tm:textMatch "words"` recalls the statements
/// whose string object matches the words and binds their eids to `?e`
/// (`lat.md/query#Text Recall`). Not a stored or virtual predicate: the front end
/// turns the group of `tm:text*` patterns on one subject into a text recall.
pub const TM_TEXT_MATCH: &str = "urn:tiramemsu:tm:textMatch";
/// `tm:textScore`: binds a text hit's lexical score.
pub const TM_TEXT_SCORE: &str = "urn:tiramemsu:tm:textScore";
/// `tm:textRank`: binds a text hit's 1-based rank.
pub const TM_TEXT_RANK: &str = "urn:tiramemsu:tm:textRank";
/// `tm:textConfidence`: binds a text hit's confidence layer (unbound when absent).
pub const TM_TEXT_CONFIDENCE: &str = "urn:tiramemsu:tm:textConfidence";
/// `tm:textLimit`: keeps the first N text hits by rank.
pub const TM_TEXT_LIMIT: &str = "urn:tiramemsu:tm:textLimit";
/// `tm:textMode`: `"all"` (default), `"any"` or `"phrase"`.
pub const TM_TEXT_MODE: &str = "urn:tiramemsu:tm:textMode";

/// `tm:arrival`: in SPARQL, `?end tm:arrival ?t` inside a `SERVICE
/// <urn:tiramemsu:tm:timeRespecting…>` scope binds the arrival of the
/// time-respecting path that ends at `?end` (`lat.md/query#Temporal Path Syntax`).
/// Not a stored or virtual predicate.
pub const TM_ARRIVAL: &str = "urn:tiramemsu:tm:arrival";

/// `tm:vectorMatch`: in SPARQL, `?e tm:vectorMatch "[0.1, …]"` recalls the
/// statements whose embedding is nearest to the vector (a JSON array of numbers in
/// a string literal); `tm:vectorModel` names the model. The group of `tm:vector*`
/// patterns on one subject turns into a vector recall (`lat.md/query#Vector Recall`).
pub const TM_VECTOR_MATCH: &str = "urn:tiramemsu:tm:vectorMatch";
/// `tm:vectorModel`: the embedding model IRI of a vector recall (required).
pub const TM_VECTOR_MODEL: &str = "urn:tiramemsu:tm:vectorModel";
/// `tm:vectorSimilarity`: binds a vector hit's similarity.
pub const TM_VECTOR_SIMILARITY: &str = "urn:tiramemsu:tm:vectorSimilarity";
/// `tm:vectorRank`: binds a vector hit's 1-based rank.
pub const TM_VECTOR_RANK: &str = "urn:tiramemsu:tm:vectorRank";
/// `tm:vectorConfidence`: binds a vector hit's confidence layer (unbound when absent).
pub const TM_VECTOR_CONFIDENCE: &str = "urn:tiramemsu:tm:vectorConfidence";
/// `tm:vectorLimit`: keeps the first N vector hits by rank (default 10).
pub const TM_VECTOR_LIMIT: &str = "urn:tiramemsu:tm:vectorLimit";

/// `tm:vectorPredicate`: keeps statements with this predicate (repeatable).
pub const TM_VECTOR_PREDICATE: &str = "urn:tiramemsu:tm:vectorPredicate";

/// The `tm:vector*` pattern predicates.
pub const VECTOR: [&str; 7] = [
    TM_VECTOR_MATCH,
    TM_VECTOR_MODEL,
    TM_VECTOR_SIMILARITY,
    TM_VECTOR_RANK,
    TM_VECTOR_CONFIDENCE,
    TM_VECTOR_LIMIT,
    TM_VECTOR_PREDICATE,
];

/// `tm:recallText`: in SPARQL, the words of a hybrid recall on the subject
/// variable; the group of `tm:recall*` patterns on one subject turns into one
/// recall (`lat.md/query#Hybrid Recall`).
pub const TM_RECALL_TEXT: &str = "urn:tiramemsu:tm:recallText";
/// `tm:recallMode`: how the words combine (`"all"`, `"any"`, `"phrase"`).
pub const TM_RECALL_MODE: &str = "urn:tiramemsu:tm:recallMode";
/// `tm:recallVector`: the query vector (a string holding a JSON array).
pub const TM_RECALL_VECTOR: &str = "urn:tiramemsu:tm:recallVector";
/// `tm:recallModel`: the embedding model IRI of the vector channel.
pub const TM_RECALL_MODEL: &str = "urn:tiramemsu:tm:recallModel";
/// `tm:recallScore`: binds the fused score.
pub const TM_RECALL_SCORE: &str = "urn:tiramemsu:tm:recallScore";
/// `tm:recallRank`: binds the 1-based rank.
pub const TM_RECALL_RANK: &str = "urn:tiramemsu:tm:recallRank";
/// `tm:recallVia`: binds the hit an expanded statement was reached from.
pub const TM_RECALL_VIA: &str = "urn:tiramemsu:tm:recallVia";
/// `tm:recallLimit`: at most N fused hits (default 10).
pub const TM_RECALL_LIMIT: &str = "urn:tiramemsu:tm:recallLimit";
/// `tm:recallExpand`: layers and subject facts added per fused hit.
pub const TM_RECALL_EXPAND: &str = "urn:tiramemsu:tm:recallExpand";

/// The `tm:recall*` pattern predicates.
pub const RECALL: [&str; 9] = [
    TM_RECALL_TEXT,
    TM_RECALL_MODE,
    TM_RECALL_VECTOR,
    TM_RECALL_MODEL,
    TM_RECALL_SCORE,
    TM_RECALL_RANK,
    TM_RECALL_VIA,
    TM_RECALL_LIMIT,
    TM_RECALL_EXPAND,
];

/// The `tm:text*` pattern predicates.
pub const TEXT: [&str; 6] = [
    TM_TEXT_MATCH,
    TM_TEXT_SCORE,
    TM_TEXT_RANK,
    TM_TEXT_CONFIDENCE,
    TM_TEXT_LIMIT,
    TM_TEXT_MODE,
];

/// Every virtual predicate IRI.
pub const VIRTUAL: [&str; 10] = [
    SYS_SUBJECT,
    SYS_PREDICATE,
    SYS_OBJECT,
    TM_TX_ADDED,
    TM_TX_RETRACTED,
    TM_ADDED_AT,
    TM_RETRACTED_AT,
    TM_VALID_FROM,
    TM_VALID_TO,
    TM_RETRACT_KIND,
];

/// True when `iri` is one of the virtual predicates. Other `sys:`/`tm:` IRIs are
/// ordinary IRIs.
pub fn is_virtual(iri: &str) -> bool {
    (iri.starts_with(SYS) || iri.starts_with(TM)) && VIRTUAL.contains(&iri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_the_table() {
        for v in VIRTUAL {
            assert!(is_virtual(v));
        }
        assert!(!is_virtual("urn:tiramemsu:sys:cardinality"));
        assert!(!is_virtual("urn:tiramemsu:tm:asOf/150"));
        assert!(!is_virtual("urn:tiramemsu:v:subject"));
    }
}
