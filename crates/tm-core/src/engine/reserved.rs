//! The reserved `sys:` / `tm:` namespaces for user-written predicates (design D-11).

use crate::error::{Error, Result};
use crate::id::Tag;
use crate::vocab;

/// Checks that a user write may use predicate `p_iri` with a subject of tag `s_tag`.
///
/// `sys:sensitive` is rejected with `Unsupported` (reserved for M6) before the
/// allow-list, so the caller learns why.
// @lat: [[data-model#Vocabulary Mapping#Reserved Namespaces]]
pub(crate) fn check_predicate(p_iri: &str, s_tag: Tag) -> Result<()> {
    if p_iri == vocab::SYS_SENSITIVE {
        return Err(Error::Unsupported {
            feature: vocab::SENSITIVE_FEATURE.to_string(),
        });
    }
    // engine-owned: only GRAPH blocks, WITH and `Tx::add_to_graph` write memberships
    if p_iri == vocab::SYS_IN_GRAPH {
        return Err(Error::ReservedNamespace(p_iri.to_string()));
    }
    if p_iri.starts_with(vocab::TM) {
        return Err(Error::ReservedNamespace(p_iri.to_string()));
    }
    if let Some(local) = p_iri.strip_prefix(vocab::SYS) {
        if vocab::SYS_ALLOWED.contains(&local) {
            return Ok(());
        }
        if vocab::SYS_TX_METADATA.contains(&local) && s_tag == Tag::Tx {
            return Ok(());
        }
        return Err(Error::ReservedNamespace(p_iri.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_list() {
        assert!(check_predicate("urn:tiramemsu:sys:unique", Tag::Iri).is_ok());
        assert!(check_predicate("urn:tiramemsu:sys:subjectType", Tag::Iri).is_ok());
        assert!(check_predicate("urn:tiramemsu:sys:prefixIri", Tag::BNode).is_ok());
        assert!(check_predicate("urn:tiramemsu:sys:reason", Tag::Tx).is_ok());
        assert!(matches!(
            check_predicate("urn:tiramemsu:sys:reason", Tag::Iri),
            Err(Error::ReservedNamespace(_))
        ));
        assert!(matches!(
            check_predicate("urn:tiramemsu:sys:confirmedBy", Tag::Stmt),
            Err(Error::ReservedNamespace(_))
        ));
        assert!(matches!(
            check_predicate("urn:tiramemsu:tm:txAdded", Tag::Stmt),
            Err(Error::ReservedNamespace(_))
        ));
        assert!(matches!(
            check_predicate("urn:tiramemsu:sys:sensitive", Tag::Iri),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            check_predicate("urn:tiramemsu:sys:inGraph", Tag::Stmt),
            Err(Error::ReservedNamespace(m)) if m == vocab::SYS_IN_GRAPH
        ));
        assert!(check_predicate("urn:tiramemsu:v:name", Tag::Iri).is_ok());
    }
}
