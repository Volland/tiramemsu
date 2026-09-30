//! Name to IRI mapping: the shared [`Vocab`] of `tm-core` plus resolution of
//! Cypher [`Name`]s with span errors (`lat.md/data-model#Vocabulary Mapping`).

pub use tm_core::mapping::{is_absolute_iri, pct_decode, pct_encode, BUILTIN_PREFIXES, RDFS};
#[cfg(test)]
use tm_core::Value;
pub use tm_core::Vocab;

use crate::ast::Name;
use crate::error::{CResult, CypherError};

/// Resolution of a Cypher [`Name`] through a [`Vocab`].
pub trait VocabExt {
    /// Resolves a [`Name`], reporting a `Parse` error at its span.
    fn resolve(&self, n: &Name) -> CResult<String>;
}

impl VocabExt for Vocab {
    fn resolve(&self, n: &Name) -> CResult<String> {
        self.resolve_text(&n.text, n.escaped)
            .map_err(|m| CypherError::parse(n.span, m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(t: &str, esc: bool) -> Name {
        Name {
            text: t.into(),
            escaped: esc,
            span: crate::Span::new(0, t.len()),
        }
    }

    fn with_schema() -> Vocab {
        Vocab {
            prefixes: vec![("schema".into(), "https://schema.org/".into())],
            ..Vocab::default()
        }
    }

    // vocabulary-mapping "Case is preserved" / "Relationship type and key"
    #[test]
    fn bare_names_use_the_vocab_verbatim() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve(&n("Person", false)).unwrap(),
            "urn:tiramemsu:v:Person"
        );
        assert_eq!(
            v.resolve(&n("person", false)).unwrap(),
            "urn:tiramemsu:v:person"
        );
        assert_eq!(
            v.resolve(&n("WORKS_AT", false)).unwrap(),
            "urn:tiramemsu:v:WORKS_AT"
        );
    }

    // "Backticked name with a space"
    #[test]
    fn backticked_space_is_percent_encoded() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve(&n("Top Customer", true)).unwrap(),
            "urn:tiramemsu:v:Top%20Customer"
        );
        assert_eq!(v.render("urn:tiramemsu:v:Top%20Customer"), "Top Customer");
    }

    // "User prefix" / "Built-in rdfs prefix" / "v prefix equals @vocab"
    #[test]
    fn curies_resolve_through_the_table() {
        let v = with_schema();
        assert_eq!(
            v.resolve(&n("schema:name", true)).unwrap(),
            "https://schema.org/name"
        );
        assert_eq!(
            v.resolve(&n("rdfs:label", true)).unwrap(),
            "http://www.w3.org/2000/01/rdf-schema#label"
        );
        assert_eq!(
            v.resolve(&n("v:Person", true)).unwrap(),
            v.resolve(&n("Person", false)).unwrap()
        );
        assert_eq!(
            v.resolve(&n("sys:unique", true)).unwrap(),
            "urn:tiramemsu:sys:unique"
        );
    }

    // "Full IRI label" / "Invalid IRI"
    #[test]
    fn full_iris_and_invalid_names() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve(&n("https://schema.org/Organization", true))
                .unwrap(),
            "https://schema.org/Organization"
        );
        assert!(v.resolve(&n("1bad:<>", true)).is_err());
    }

    // "Local name" / "CURIE and full IRI" / "Local name containing a colon is not shortened"
    #[test]
    fn rendering() {
        let v = with_schema();
        assert_eq!(v.render("urn:tiramemsu:v:Person"), "Person");
        assert_eq!(v.render("https://schema.org/Person"), "schema:Person");
        assert_eq!(v.render("http://example.org/X"), "http://example.org/X");
        assert_eq!(v.render("urn:tiramemsu:v:schema:name"), "v:schema:name");
        assert_eq!(v.render("urn:tiramemsu:sys:unique"), "sys:unique");
    }

    // longest prefix wins, ties by smallest name
    #[test]
    fn longest_prefix_and_tie_break() {
        let v = Vocab {
            prefixes: vec![
                ("b".into(), "https://x.org/a/".into()),
                ("a".into(), "https://x.org/a/".into()),
                ("short".into(), "https://x.org/".into()),
            ],
            ..Vocab::default()
        };
        assert_eq!(v.render("https://x.org/a/n"), "a:n");
        assert_eq!(v.render("https://x.org/z"), "short:z");
    }

    // "Round trip of rendered labels"
    #[test]
    fn names_round_trip() {
        let v = with_schema();
        for iri in [
            "urn:tiramemsu:v:Person",
            "urn:tiramemsu:v:Top%20Customer",
            "https://schema.org/Person",
            "http://example.org/X",
            "urn:tiramemsu:v:schema:name",
            "urn:tiramemsu:sys:supersedes",
        ] {
            let r = v.render(iri);
            let back = v.resolve_text(&r, true).unwrap();
            assert_eq!(back, iri, "rendered as {r}");
        }
    }

    #[test]
    fn ids() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve_id("v:alice").unwrap(),
            Value::iri("urn:tiramemsu:v:alice")
        );
        assert_eq!(
            v.resolve_id("urn:tiramemsu:node:12").unwrap(),
            Value::Node(12)
        );
        assert_eq!(
            v.resolve_id("urn:tiramemsu:bnode:3").unwrap(),
            Value::BNode(3)
        );
        assert!(matches!(
            v.resolve_id("urn:tiramemsu:stmt:7").unwrap(),
            Value::Stmt(_)
        ));
        assert!(v.resolve_id("not an iri").is_err());
    }
}
