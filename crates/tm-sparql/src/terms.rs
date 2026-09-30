//! spargebra terms ↔ store values, and the skolem IRI forms (design D9).

use spargebra::term::{Literal, NamedNode};
use tm_core::{codec, Result, Tag, Value};

use crate::error::unsupported;

/// The canonical value of an IRI. Skolem IRIs (`urn:tiramemsu:node:<n>`, `bnode:`,
/// `stmt:`, `tx:`) become the node, blank node, statement or transaction they name.
pub fn named_node(n: &NamedNode) -> Value {
    Value::Iri(n.as_str().to_string()).canonical()
}

/// The canonical value of an RDF literal (`xsd:integer` collapses to its value,
/// a date-time keeps its offset, language tags are lower-cased).
pub fn literal(l: &Literal) -> Result<Value> {
    if l.direction().is_some() {
        return Err(unsupported("directional language-tagged string"));
    }
    if let Some(lang) = l.language() {
        return Ok(Value::literal(l.value(), None, Some(lang)));
    }
    Ok(Value::literal(l.value(), Some(l.datatype().as_str()), None))
}

/// The inline ObjectId of an `INT`/`BOOL`/`DATETIME`/`DATE`/skolem value.
pub fn inline_id(v: &Value) -> Option<tm_core::ObjectId> {
    match codec::encode(v) {
        codec::Encoded::Inline(id) => Some(id),
        codec::Encoded::Term(_) => None,
    }
}

/// The ObjectId a skolem IRI denotes (`NODE`, `BNODE`, `STMT` or `TX`).
pub fn parse_skolem(iri: &str) -> Option<tm_core::ObjectId> {
    codec::parse_skolem(iri).and_then(|v| inline_id(&v))
}

/// The skolem IRI of a `NODE`, `BNODE`, `STMT` or `TX` id.
pub fn format_skolem(id: tm_core::ObjectId) -> Option<String> {
    codec::skolem_iri(id)
}

/// True for the four skolem tags.
pub fn is_skolem_tag(t: Tag) -> bool {
    matches!(t, Tag::Node | Tag::BNode | Tag::Stmt | Tag::Tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tm_core::vocab::{XSD_DATETIME, XSD_INTEGER};
    use tm_core::ObjectId;

    fn lit(lex: &str, dt: &str) -> Value {
        literal(&Literal::new_typed_literal(
            lex,
            NamedNode::new_unchecked(dt),
        ))
        .unwrap()
    }

    // data-model "ObjectId#Canonical Encoding": integer lexical forms collapse
    #[test]
    fn integer_lexical_forms_are_equal() {
        assert_eq!(lit("01", XSD_INTEGER), Value::Int(1));
        assert_eq!(lit("+7", XSD_INTEGER), Value::Int(7));
    }

    // tests "ObjectId#DateTime Keeps Its Offset"
    #[test]
    fn datetime_offsets_are_distinct_terms_with_one_instant() {
        let a = lit("2026-03-01T12:00:00+02:00", XSD_DATETIME);
        let b = lit("2026-03-01T10:00:00Z", XSD_DATETIME);
        let c = lit("2026-03-01T10:00:00", XSD_DATETIME);
        let (ia, ib, ic) = (
            inline_id(&a).unwrap(),
            inline_id(&b).unwrap(),
            inline_id(&c).unwrap(),
        );
        assert_ne!(ia, ib);
        assert_ne!(ib, ic);
        assert_eq!(ia.raw() >> 15, ib.raw() >> 15);
        assert_eq!(ib.raw() >> 15, ic.raw() >> 15);
        // no timezone: tz code 0
        assert_eq!((ic.raw() >> 4) & 0x7ff, 0);
        // each decodes to its own lexical offset
        assert_eq!(a.lexical(), "2026-03-01T12:00:00.000+02:00");
        assert_eq!(b.lexical(), "2026-03-01T10:00:00.000Z");
        assert_eq!(c.lexical(), "2026-03-01T10:00:00.000");
    }

    #[test]
    fn language_tags_are_lower_cased() {
        let l = Literal::new_language_tagged_literal("x", "DE").unwrap();
        assert_eq!(
            literal(&l).unwrap(),
            Value::LangStr {
                lex: "x".into(),
                lang: "de".into()
            }
        );
    }

    #[test]
    fn skolem_iris_name_their_ids() {
        assert_eq!(
            named_node(&NamedNode::new_unchecked("urn:tiramemsu:stmt:12")),
            Value::Stmt(tm_core::Eid::new(12))
        );
        assert_eq!(
            named_node(&NamedNode::new_unchecked("urn:tiramemsu:v:x")),
            Value::iri("urn:tiramemsu:v:x")
        );
        // non-canonical numbers are ordinary IRIs
        assert!(parse_skolem("urn:tiramemsu:node:007").is_none());
    }

    // one property test per tag: format → parse round-trips
    proptest! {
        #[test]
        fn skolem_round_trips(n in 1u64..(1u64 << 60), tag in prop::sample::select(vec![Tag::Node, Tag::BNode, Tag::Stmt, Tag::Tx])) {
            let id = ObjectId::from_unsigned(tag, n);
            let iri = format_skolem(id).unwrap();
            prop_assert_eq!(parse_skolem(&iri), Some(id));
        }
    }
}
