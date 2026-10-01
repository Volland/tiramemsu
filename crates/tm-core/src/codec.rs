//! Inline encoding and decoding of ObjectIds, and skolem IRIs.
//!
//! Values that fit in 60 bits are encoded inline; the rest become a
//! [`TermSpec`] that the term dictionary stores.

// @lat: [[data-model#ObjectId#Canonical Encoding]]

use crate::error::{Error, Position, Result};
use crate::id::{Eid, ObjectId, Tag, TxId};
use crate::value::{self, Value};
use crate::vocab::{self, XSD_DECIMAL, XSD_DOUBLE};

/// Maximum byte length of an inline `SHORT_STR`.
pub const SHORT_STR_MAX: usize = 7;

/// Timezone code for an offset: 0 = none, else minutes + 841.
pub fn tz_code(tz: Option<i16>) -> i64 {
    match tz {
        None => 0,
        Some(m) => m as i64 + 841,
    }
}

/// Offset in minutes for a timezone code.
pub fn tz_from_code(code: i64) -> Result<Option<i16>> {
    match code {
        0 => Ok(None),
        1..=1681 => Ok(Some((code - 841) as i16)),
        _ => Err(Error::InvalidTerm {
            position: Position::Value,
            reason: format!("timezone code {code} out of range"),
        }),
    }
}

/// A value that needs the term dictionary.
#[derive(Clone, Debug, PartialEq)]
pub struct TermSpec {
    /// One of `IRI`, `STR`, `LANG_STR`, `TYPED`, `DOUBLE`, `DECIMAL`.
    pub tag: Tag,
    /// Canonical lexical form.
    pub lex: String,
    /// Datatype IRI (for `TYPED`, `DOUBLE`, `DECIMAL`).
    pub datatype: Option<String>,
    /// Lower-cased language tag (for `LANG_STR`).
    pub lang: Option<String>,
    /// Numeric value (for `DOUBLE` and `DECIMAL`; `None` for NaN).
    pub num: Option<f64>,
}

/// The encoding of a value: an inline id, or a term to look up or intern.
#[derive(Clone, Debug, PartialEq)]
pub enum Encoded {
    /// Fully encoded without the dictionary.
    Inline(ObjectId),
    /// Needs a dictionary id.
    Term(TermSpec),
}

impl Encoded {
    /// Fails with `Unsupported` for an inline `NODE`, `BNODE`, `STMT` or `TX` id
    /// with a non-zero origin (see [`ObjectId::check_origin`]).
    pub fn check_origin(&self) -> Result<()> {
        match self {
            Encoded::Inline(id) => id.check_origin(),
            Encoded::Term(_) => Ok(()),
        }
    }
}

/// Encodes a value, canonicalising it first.
pub fn encode(v: &Value) -> Encoded {
    match v.canonical() {
        Value::Iri(s) => Encoded::Term(TermSpec {
            tag: Tag::Iri,
            lex: s,
            datatype: None,
            lang: None,
            num: None,
        }),
        Value::Node(n) => Encoded::Inline(ObjectId::from_unsigned(Tag::Node, n)),
        Value::BNode(n) => Encoded::Inline(ObjectId::from_unsigned(Tag::BNode, n)),
        Value::Stmt(e) => Encoded::Inline(e.oid()),
        Value::Tx(t) => Encoded::Inline(t.oid()),
        Value::Int(i) => Encoded::Inline(ObjectId::from_signed(Tag::Int, i)),
        Value::Bool(b) => Encoded::Inline(ObjectId::from_unsigned(Tag::Bool, b as u64)),
        Value::DateTime { ms, tz } => Encoded::Inline(ObjectId::from_signed(
            Tag::DateTime,
            (ms << 11) | tz_code(tz),
        )),
        Value::Date(d) => Encoded::Inline(ObjectId::from_signed(Tag::Date, d)),
        Value::Str(s) if s.len() <= SHORT_STR_MAX => {
            Encoded::Inline(ObjectId::from_unsigned(Tag::ShortStr, pack_short(&s)))
        }
        Value::Str(s) => Encoded::Term(TermSpec {
            tag: Tag::Str,
            lex: s,
            datatype: None,
            lang: None,
            num: None,
        }),
        Value::LangStr { lex, lang } => Encoded::Term(TermSpec {
            tag: Tag::LangStr,
            lex,
            datatype: None,
            lang: Some(lang),
            num: None,
        }),
        Value::Typed { lex, datatype } => Encoded::Term(TermSpec {
            tag: Tag::Typed,
            lex,
            datatype: Some(datatype),
            lang: None,
            num: None,
        }),
        Value::Double(x) => Encoded::Term(TermSpec {
            tag: Tag::Double,
            lex: value::canonical_double(x),
            datatype: Some(XSD_DOUBLE.to_string()),
            lang: None,
            num: (!x.is_nan()).then_some(x),
        }),
        Value::Decimal(s) => {
            let num = s.parse::<f64>().ok();
            Encoded::Term(TermSpec {
                tag: Tag::Decimal,
                lex: s,
                datatype: Some(XSD_DECIMAL.to_string()),
                lang: None,
                num,
            })
        }
    }
}

fn pack_short(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut p: u64 = 0;
    for i in 0..SHORT_STR_MAX {
        p = (p << 8) | *b.get(i).unwrap_or(&0) as u64;
    }
    (p << 4) | b.len() as u64
}

fn unpack_short(p: u64) -> Result<String> {
    let len = (p & 15) as usize;
    if len > SHORT_STR_MAX {
        return Err(Error::InvalidTerm {
            position: Position::Value,
            reason: format!("SHORT_STR length {len}"),
        });
    }
    let bytes = (p >> 4).to_be_bytes();
    // the 56 data bits occupy the low 7 bytes of the shifted value
    let data = &bytes[1..1 + len];
    String::from_utf8(data.to_vec()).map_err(|_| Error::InvalidTerm {
        position: Position::Value,
        reason: "SHORT_STR is not UTF-8".to_string(),
    })
}

/// Decodes an inline id. Returns `Ok(None)` for dictionary tags and
/// `Unsupported` for the reserved tag 15.
pub fn decode_inline(id: ObjectId) -> Result<Option<Value>> {
    Ok(Some(match id.tag()? {
        Tag::Node => Value::Node(id.unsigned_payload()),
        Tag::BNode => Value::BNode(id.unsigned_payload()),
        Tag::Stmt => Value::Stmt(Eid::from_oid(id).expect("STMT tag")),
        Tag::Tx => Value::Tx(TxId(id.unsigned_payload())),
        Tag::Int => Value::Int(id.signed_payload()),
        Tag::Bool => match id.unsigned_payload() {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            p => {
                return Err(Error::InvalidTerm {
                    position: Position::Value,
                    reason: format!("BOOL payload {p}"),
                })
            }
        },
        Tag::DateTime => {
            let p = id.signed_payload();
            Value::DateTime {
                ms: p >> 11,
                tz: tz_from_code(p & 0x7ff)?,
            }
        }
        Tag::Date => Value::Date(id.signed_payload()),
        Tag::ShortStr => Value::Str(unpack_short(id.unsigned_payload())?),
        _ => return Ok(None),
    }))
}

/// Builds the value of a dictionary term.
pub fn value_from_term(
    tag: Tag,
    lex: String,
    datatype: Option<String>,
    lang: Option<String>,
) -> Result<Value> {
    Ok(match tag {
        Tag::Iri => Value::Iri(lex),
        Tag::Str => Value::Str(lex),
        Tag::LangStr => Value::LangStr {
            lex,
            lang: lang.unwrap_or_default(),
        },
        Tag::Typed => Value::Typed {
            lex,
            datatype: datatype.unwrap_or_default(),
        },
        Tag::Double => Value::Double(value::parse_double(&lex).unwrap_or(f64::NAN)),
        Tag::Decimal => Value::Decimal(lex),
        other => {
            return Err(Error::InvalidTerm {
                position: Position::Value,
                reason: format!("tag {} is not a dictionary tag", other.name()),
            })
        }
    })
}

fn parse_canonical_n(s: &str) -> Option<u64> {
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = s.parse().ok()?;
    (n < (1u64 << 60)).then_some(n)
}

/// Recognises a skolem IRI (`urn:tiramemsu:node:<n>`, `bnode:`, `stmt:`, `tx:`)
/// with a canonical decimal `1 <= n < 2^60`.
pub fn parse_skolem(iri: &str) -> Option<Value> {
    if let Some(n) = iri.strip_prefix(vocab::SKOLEM_NODE) {
        return parse_canonical_n(n).map(Value::Node);
    }
    if let Some(n) = iri.strip_prefix(vocab::SKOLEM_BNODE) {
        return parse_canonical_n(n).map(Value::BNode);
    }
    if let Some(n) = iri.strip_prefix(vocab::SKOLEM_STMT) {
        return parse_canonical_n(n).map(|n| Value::Stmt(Eid::new(n)));
    }
    if let Some(n) = iri.strip_prefix(vocab::SKOLEM_TX) {
        return parse_canonical_n(n).map(|n| Value::Tx(TxId(n)));
    }
    None
}

/// The skolem IRI of a `NODE`, `BNODE`, `STMT` or `TX` id.
pub fn skolem_iri(id: ObjectId) -> Option<String> {
    let n = id.unsigned_payload();
    match id.tag().ok()? {
        Tag::Node => Some(format!("{}{n}", vocab::SKOLEM_NODE)),
        Tag::BNode => Some(format!("{}{n}", vocab::SKOLEM_BNODE)),
        Tag::Stmt => Some(format!("{}{n}", vocab::SKOLEM_STMT)),
        Tag::Tx => Some(format!("{}{n}", vocab::SKOLEM_TX)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocab::*;

    fn inline(v: Value) -> ObjectId {
        match encode(&v) {
            Encoded::Inline(id) => id,
            e => panic!("not inline: {e:?}"),
        }
    }

    fn term(v: Value) -> TermSpec {
        match encode(&v) {
            Encoded::Term(t) => t,
            e => panic!("not a term: {e:?}"),
        }
    }

    #[test]
    fn integers() {
        let one = inline(Value::Int(1));
        for lex in ["01", "+1", "1"] {
            assert_eq!(inline(Value::literal(lex, Some(XSD_INTEGER), None)), one);
        }
        assert_eq!(inline(Value::Int(5)).raw(), 85);
        let max = (1i64 << 59) - 1;
        assert_eq!(inline(Value::Int(max)).tag().unwrap(), Tag::Int);
        assert_eq!(inline(Value::Int(-(1i64 << 59))).tag().unwrap(), Tag::Int);
        let t = term(Value::Int(1i64 << 59));
        assert_eq!((t.tag, t.lex.as_str()), (Tag::Typed, "576460752303423488"));
        assert_eq!(t.datatype.as_deref(), Some(XSD_INTEGER));
        let t = term(Value::big_integer("-576460752303423489"));
        assert_eq!((t.tag, t.lex.as_str()), (Tag::Typed, "-576460752303423489"));
        let t = term(Value::literal(
            "5",
            Some("http://www.w3.org/2001/XMLSchema#int"),
            None,
        ));
        assert_eq!(t.tag, Tag::Typed);
        let t = term(Value::literal("abc", Some(XSD_INTEGER), None));
        assert_eq!((t.tag, t.lex.as_str()), (Tag::Typed, "abc"));
    }

    #[test]
    fn booleans_and_dates() {
        let t = inline(Value::literal("1", Some(XSD_BOOLEAN), None));
        assert_eq!(t, inline(Value::literal("true", Some(XSD_BOOLEAN), None)));
        assert_eq!((t.tag().unwrap(), t.unsigned_payload()), (Tag::Bool, 1));
        let f = inline(Value::literal("0", Some(XSD_BOOLEAN), None));
        assert_eq!(f, inline(Value::literal("false", Some(XSD_BOOLEAN), None)));
        assert_eq!(f.unsigned_payload(), 0);
        let d = inline(Value::literal("1969-12-31", Some(XSD_DATE), None));
        assert_eq!((d.tag().unwrap(), d.signed_payload()), (Tag::Date, -1));
        let dz = inline(Value::literal("1969-12-31Z", Some(XSD_DATE), None));
        assert_eq!(d, dz);
    }

    #[test]
    fn short_strings() {
        let a = inline(Value::str("abcdefg"));
        assert_eq!(a.tag().unwrap(), Tag::ShortStr);
        assert_eq!(term(Value::str("abcdefgh")).tag, Tag::Str);
        assert_eq!(inline(Value::str("héllo")).tag().unwrap(), Tag::ShortStr);
        assert_eq!(term(Value::str("€€€")).tag, Tag::Str);
        for s in ["", "a\0b", "abcdefg", "héllo", "\u{7f}\u{7f}"] {
            let id = inline(Value::str(s));
            assert_eq!(decode_inline(id).unwrap(), Some(Value::str(s)));
        }
        assert_eq!(
            inline(Value::str("hello")),
            inline(Value::literal("hello", Some(XSD_STRING), None))
        );
    }

    #[test]
    fn skolems() {
        assert_eq!(
            inline(Value::iri("urn:tiramemsu:node:12")),
            inline(Value::Node(12))
        );
        assert_eq!(
            inline(Value::iri("urn:tiramemsu:bnode:3")),
            inline(Value::BNode(3))
        );
        assert_eq!(term(Value::iri("urn:tiramemsu:node:007")).tag, Tag::Iri);
        assert_eq!(term(Value::iri("urn:tiramemsu:node:0")).tag, Tag::Iri);
        assert_eq!(term(Value::iri("urn:tiramemsu:node:-1")).tag, Tag::Iri);
        assert_eq!(
            term(Value::iri("urn:tiramemsu:node:1152921504606846976")).tag,
            Tag::Iri
        );
        let n = inline(Value::Node(12));
        assert_eq!(skolem_iri(n).unwrap(), "urn:tiramemsu:node:12");
    }
}
