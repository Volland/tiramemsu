//! N-Triples / RDF 1.2 N-Triples writer for `CONSTRUCT` results.
//!
//! The writer lives in [`tm_core::rdf`] (no parser needed to serialise); this
//! module re-exports it under its historical name.

pub use tm_core::rdf::write_ntriples as write;

#[cfg(test)]
mod tests {
    use super::*;
    use oxttl::NTriplesParser;
    use tm_core::rdf::{RdfTerm, RdfTriple};

    fn iri_t(s: &str) -> RdfTerm {
        RdfTerm::Iri(s.to_string())
    }

    fn sample() -> Vec<RdfTriple> {
        let base = RdfTriple {
            s: iri_t("urn:tiramemsu:v:alice"),
            p: iri_t("urn:tiramemsu:v:worksAt"),
            o: iri_t("urn:tiramemsu:v:acme"),
        };
        vec![
            base.clone(),
            RdfTriple {
                s: iri_t("urn:tiramemsu:stmt:12"),
                p: iri_t("http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies"),
                o: RdfTerm::Triple(Box::new(base)),
            },
            RdfTriple {
                s: RdfTerm::Blank("b0_x".into()),
                p: iri_t("urn:tiramemsu:v:note"),
                o: RdfTerm::Literal {
                    lex: "a \"q\"\n\\".into(),
                    datatype: None,
                    lang: Some("en".into()),
                },
            },
            RdfTriple {
                s: iri_t("urn:tiramemsu:stmt:12"),
                p: iri_t("urn:tiramemsu:v:confidence"),
                o: RdfTerm::Literal {
                    lex: "0.8".into(),
                    datatype: Some("http://www.w3.org/2001/XMLSchema#decimal".into()),
                    lang: None,
                },
            },
        ]
    }

    #[test]
    fn output_parses_back_as_rdf12_ntriples() {
        let text = write(&sample());
        let parsed: Vec<_> = NTriplesParser::new()
            .for_reader(text.as_bytes())
            .collect::<Result<_, _>>()
            .expect("valid RDF 1.2 N-Triples");
        assert_eq!(parsed.len(), 4);
        assert!(text.contains(
            "<<( <urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> <urn:tiramemsu:v:acme> )>>"
        ));
    }

    #[test]
    fn plain_triples_use_plain_ntriples() {
        let text = write(&sample()[..1]);
        assert_eq!(
            text,
            "<urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> <urn:tiramemsu:v:acme> .\n"
        );
    }
}
