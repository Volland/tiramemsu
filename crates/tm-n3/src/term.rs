//! Values to N3 terms and back, through the RDF rendering of fact bundles.

use std::collections::BTreeMap;
use std::fmt;

use eyeron::{Literal, Term as NTerm};
use tm_core::rdf::{self, RdfTerm};
use tm_core::Value;

/// A term of a conclusion. Conclusions are made of stored values except where
/// the reasoner invented a node or built a term the store has no value for.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    /// A stored value: IRIs and literals, `urn:tiramemsu:node|bnode|stmt:N` mapped
    /// back to their nodes and statements.
    Value(Value),
    /// A blank node the rule invented (an existential of its conclusion), with
    /// eyeron's deterministic witness label: the same rule firing with the same
    /// bindings gives the same label, so materialization keys its Skolem node on
    /// it.
    Blank(String),
    /// An N3 list (no stored counterpart).
    List(Vec<Term>),
    /// A quoted formula, as N3 text (no stored counterpart).
    Formula(String),
}

impl Term {
    /// The stored value, if this term is one.
    pub fn as_value(&self) -> Option<&Value> {
        match self {
            Term::Value(v) => Some(v),
            _ => None,
        }
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Term::Value(v) => write!(f, "{v}"),
            Term::Blank(b) => write!(f, "_:{b}"),
            Term::List(items) => {
                f.write_str("(")?;
                for i in items {
                    write!(f, " {i}")?;
                }
                f.write_str(" )")
            }
            Term::Formula(text) => write!(f, "{{ {text} }}"),
        }
    }
}

/// The N3 term of a stored value ([`tm_core::rdf::render`]).
// @tg: implements:: [[openspec:n3-reasoning#Statements in N3]] {since: "2026-10-28"}
pub fn to_n3(v: &Value) -> NTerm {
    match rdf::render(v) {
        RdfTerm::Iri(s) => NTerm::iri(s),
        RdfTerm::Blank(b) => NTerm::blank(b),
        RdfTerm::Literal {
            lex,
            datatype,
            lang,
        } => NTerm::Literal(Literal {
            value: lex.into(),
            datatype: datatype.map(Into::into),
            language: lang.map(Into::into),
        }),
        // values never render to triple terms
        RdfTerm::Triple(t) => NTerm::iri(rdf::write_term(&RdfTerm::Triple(t))),
    }
}

/// The value of an N3 IRI or literal, `None` for other terms.
pub fn value_of(t: &NTerm) -> Option<Value> {
    match t {
        NTerm::Iri(s) => Some(Value::iri(s.as_str()).canonical()),
        NTerm::Literal(l) => Some(match &l.language {
            Some(lang) => Value::literal(&l.value, None, Some(lang)),
            None => Value::literal(&l.value, l.datatype.as_deref(), None),
        }),
        _ => None,
    }
}

/// The conclusion term of an N3 term.
pub fn from_n3(t: &NTerm) -> Term {
    if let Some(v) = value_of(t) {
        return Term::Value(v);
    }
    match t {
        NTerm::Blank(b) | NTerm::Var(b) => Term::Blank(b.to_string()),
        NTerm::List(items) => Term::List(items.iter().map(from_n3).collect()),
        other => Term::Formula(eyeron::printing::term_to_n3_object(other, &BTreeMap::new())),
    }
}

/// A canonical text key of a triple: equal for triples whose terms denote the
/// same stored values (`5` and `"05"^^xsd:integer`, `1.5e0` and `1.5E0`).
pub(crate) fn key(t: &eyeron::Triple) -> String {
    format!("{} {} {}", from_n3(&t.s), from_n3(&t.p), from_n3(&t.o))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{Eid, TxId};

    #[test]
    fn values_round_trip() {
        let values = [
            Value::iri("urn:tiramemsu:v:alice"),
            Value::Node(7),
            Value::BNode(3),
            Value::Stmt(Eid::new(12)),
            Value::Tx(TxId(4)),
            Value::Int(5),
            Value::Bool(true),
            Value::Double(1.5),
            Value::Decimal("2.5".into()),
            Value::str("hello"),
            Value::LangStr {
                lex: "hallo".into(),
                lang: "de".into(),
            },
            Value::literal(
                "2020-01-01T00:00:00Z",
                Some(tm_core::vocab::XSD_DATETIME),
                None,
            ),
            Value::Typed {
                lex: "x".into(),
                datatype: "urn:dt".into(),
            },
        ];
        for v in values {
            let t = to_n3(&v);
            assert_eq!(from_n3(&t), Term::Value(v.clone()), "{t:?}");
        }
        assert_eq!(
            to_n3(&Value::Stmt(Eid::new(12))),
            NTerm::iri("urn:tiramemsu:stmt:12")
        );
        assert_eq!(to_n3(&Value::Node(7)), NTerm::iri("urn:tiramemsu:node:7"));
        assert_eq!(to_n3(&Value::BNode(3)), NTerm::iri("urn:tiramemsu:bnode:3"));
    }

    #[test]
    fn numeric_spellings_share_a_key() {
        let five = |lex: &str| {
            eyeron::Triple::new(
                NTerm::iri("urn:a"),
                NTerm::iri("urn:p"),
                NTerm::Literal(Literal {
                    value: lex.into(),
                    datatype: Some(tm_core::vocab::XSD_INTEGER.into()),
                    language: None,
                }),
            )
        };
        assert_eq!(key(&five("5")), key(&five("05")));
    }
}
