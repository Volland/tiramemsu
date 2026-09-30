//! N-Triples / RDF 1.2 N-Triples writer for `CONSTRUCT` results.

use std::fmt::Write;

use super::term::{RdfTerm, RdfTriple};

fn iri(s: &str, out: &mut String) {
    out.push('<');
    for c in s.chars() {
        match c {
            '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\' | ' ' | '\n' | '\r' | '\t' => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('>');
}

fn literal(lex: &str, out: &mut String) {
    out.push('"');
    for c in lex.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn term(t: &RdfTerm, out: &mut String) {
    match t {
        RdfTerm::Iri(s) => iri(s, out),
        RdfTerm::Blank(b) => {
            out.push_str("_:");
            out.push_str(b);
        }
        RdfTerm::Literal {
            lex,
            datatype,
            lang,
        } => {
            literal(lex, out);
            if let Some(l) = lang {
                out.push('@');
                out.push_str(l);
            } else if let Some(d) = datatype {
                out.push_str("^^");
                iri(d, out);
            }
        }
        RdfTerm::Triple(tr) => {
            out.push_str("<<( ");
            triple_body(tr, out);
            out.push_str(" )>>");
        }
    }
}

fn triple_body(t: &RdfTriple, out: &mut String) {
    term(&t.s, out);
    out.push(' ');
    term(&t.p, out);
    out.push(' ');
    term(&t.o, out);
}

/// N-Triples text of `triples`, one per line. RDF 1.2 triple-term syntax
/// (`<<( s p o )>>`) is used exactly where a triple term occurs.
pub fn write(triples: &[RdfTriple]) -> String {
    let mut out = String::new();
    for t in triples {
        triple_body(t, &mut out);
        out.push_str(" .\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxttl::NTriplesParser;

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
