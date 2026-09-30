//! IRIs of the virtual predicates (`lat.md/query#Views and Scans#Virtual Predicates`).

use tm_core::vocab::{SYS, TM};

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
/// `tm:validFrom`: the start of the statement's valid interval.
pub const TM_VALID_FROM: &str = "urn:tiramemsu:tm:validFrom";
/// `tm:validTo`: the end of the statement's valid interval.
pub const TM_VALID_TO: &str = "urn:tiramemsu:tm:validTo";
/// `tm:retractKind`: 0 explicit, 1 cascade, 2 supersede, 3 cardinality.
pub const TM_RETRACT_KIND: &str = "urn:tiramemsu:tm:retractKind";

/// Every virtual predicate IRI.
pub const VIRTUAL: [&str; 8] = [
    SYS_SUBJECT,
    SYS_PREDICATE,
    SYS_OBJECT,
    TM_TX_ADDED,
    TM_TX_RETRACTED,
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
