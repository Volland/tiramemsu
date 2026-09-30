//! ObjectId values → RDF terms (design D9).

use tm_core::{value, vocab, Value};

/// An RDF term as returned to callers: what a stored [`Value`] looks like in
/// SPARQL JSON and N-Triples.
///
/// Nodes, blank nodes, statements and transactions are IRIs of the form
/// `urn:tiramemsu:node:<n>`, `bnode:<n>`, `stmt:<n>` and `tx:<t>`. Written back in
/// a later query they resolve to the same ids.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RdfTerm {
    /// An IRI (skolem IRIs for nodes, blank nodes, statements and transactions).
    Iri(String),
    /// A blank node (only in `CONSTRUCT` output).
    Blank(String),
    /// A literal. `datatype` is `None` for plain strings and language strings.
    Literal {
        /// The lexical form.
        lex: String,
        /// The datatype IRI.
        datatype: Option<String>,
        /// The lower-cased language tag.
        lang: Option<String>,
    },
    /// A triple term `<<( s p o )>>`.
    Triple(Box<RdfTriple>),
}

/// An RDF triple.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RdfTriple {
    /// Subject.
    pub s: RdfTerm,
    /// Predicate.
    pub p: RdfTerm,
    /// Object.
    pub o: RdfTerm,
}

impl RdfTerm {
    fn typed(lex: String, dt: &str) -> RdfTerm {
        RdfTerm::Literal {
            lex,
            datatype: Some(dt.to_string()),
            lang: None,
        }
    }

    /// True for IRIs.
    pub fn is_iri(&self) -> bool {
        matches!(self, RdfTerm::Iri(_))
    }
}

/// A date-time in its stored offset: local time plus `±hh:mm`, `Z` for a zero
/// offset, no suffix without a timezone, and `.sss` only when the milliseconds
/// are non-zero.
pub fn datetime_lexical(ms: i64, tz: Option<i16>) -> String {
    let full = value::format_datetime(ms, tz);
    // YYYY-MM-DDThh:mm:ss.mmm[zone]: drop a zero fraction
    match full.find('.') {
        Some(i) if full[i + 1..].starts_with("000") => {
            format!("{}{}", &full[..i], &full[i + 4..])
        }
        _ => full,
    }
}

/// The RDF term of a stored value.
pub fn render(v: &Value) -> RdfTerm {
    match v {
        Value::Iri(s) => RdfTerm::Iri(s.clone()),
        Value::Node(_) | Value::BNode(_) | Value::Stmt(_) | Value::Tx(_) => {
            RdfTerm::Iri(v.lexical())
        }
        Value::Int(i) => RdfTerm::typed(i.to_string(), vocab::XSD_INTEGER),
        Value::Bool(b) => RdfTerm::typed(b.to_string(), vocab::XSD_BOOLEAN),
        Value::DateTime { ms, tz } => {
            RdfTerm::typed(datetime_lexical(*ms, *tz), vocab::XSD_DATETIME)
        }
        Value::Date(d) => RdfTerm::typed(value::format_date(*d), vocab::XSD_DATE),
        Value::Str(s) => RdfTerm::Literal {
            lex: s.clone(),
            datatype: None,
            lang: None,
        },
        Value::LangStr { lex, lang } => RdfTerm::Literal {
            lex: lex.clone(),
            datatype: None,
            lang: Some(lang.to_ascii_lowercase()),
        },
        Value::Typed { lex, datatype } => RdfTerm::typed(lex.clone(), datatype),
        Value::Double(_) => RdfTerm::typed(v.lexical(), vocab::XSD_DOUBLE),
        Value::Decimal(s) => RdfTerm::typed(s.clone(), vocab::XSD_DECIMAL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(ms: i64, tz: Option<i16>) -> String {
        match render(&Value::DateTime { ms, tz }) {
            RdfTerm::Literal { lex, .. } => lex,
            other => panic!("{other:?}"),
        }
    }

    fn ms(text: &str) -> (i64, Option<i16>) {
        value::parse_datetime(text).unwrap()
    }

    // sparql-query "Result terms": Date-time rendering
    #[test]
    fn datetime_rendering() {
        let (m, tz) = ms("2026-09-01T12:00:00.250Z");
        assert_eq!(dt(m, tz), "2026-09-01T12:00:00.250Z");
        let (m, tz) = ms("2026-09-01T12:00:00.000Z");
        assert_eq!(dt(m, tz), "2026-09-01T12:00:00Z");
    }

    // sparql-query "Result terms": Date-time rendering keeps the offset
    #[test]
    fn datetime_rendering_keeps_the_offset() {
        let (m, tz) = ms("2026-09-01T14:00:00.000+02:00");
        assert_eq!(dt(m, tz), "2026-09-01T14:00:00+02:00");
        let (m, tz) = ms("2026-09-01T12:00:00+00:00");
        assert_eq!(dt(m, tz), "2026-09-01T12:00:00Z");
        let (m, tz) = ms("2026-09-01T12:00:00");
        assert_eq!(dt(m, tz), "2026-09-01T12:00:00");
        let (m, tz) = ms("2026-09-01T09:00:00-03:30");
        assert_eq!(dt(m, tz), "2026-09-01T09:00:00-03:30");
    }

    #[test]
    fn term_table() {
        assert_eq!(
            render(&Value::Stmt(tm_core::Eid::new(12))),
            RdfTerm::Iri("urn:tiramemsu:stmt:12".into())
        );
        assert_eq!(
            render(&Value::Node(7)),
            RdfTerm::Iri("urn:tiramemsu:node:7".into())
        );
        assert_eq!(
            render(&Value::BNode(3)),
            RdfTerm::Iri("urn:tiramemsu:bnode:3".into())
        );
        assert_eq!(
            render(&Value::Tx(tm_core::TxId(9))),
            RdfTerm::Iri("urn:tiramemsu:tx:9".into())
        );
        assert_eq!(
            render(&Value::LangStr {
                lex: "Hallo".into(),
                lang: "DE".into()
            }),
            RdfTerm::Literal {
                lex: "Hallo".into(),
                datatype: None,
                lang: Some("de".into())
            }
        );
        assert_eq!(
            render(&Value::Str("x".into())),
            RdfTerm::Literal {
                lex: "x".into(),
                datatype: None,
                lang: None
            }
        );
        assert!(
            matches!(render(&Value::Date(0)), RdfTerm::Literal { lex, .. } if lex == "1970-01-01")
        );
    }
}
