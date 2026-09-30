//! Virtual predicates: predicates computed from the statement's own row
//! (`lat.md/query#Views and Scans#Virtual Predicates`, design D6).
//!
//! | IRI | Column | Object (Term domain) | Constant object compare |
//! |---|---|---|---|
//! | `sys:subject` / `sys:object` | `s` / `o` | `a.s` / `a.o` | `a.s = ?id` |
//! | `sys:predicate` | `p` | `a.p` | `a.p = ?id` (an IRI) |
//! | `tm:txAdded` / `tm:txRetracted` | `t_add` / `t_ret` | `((a.t_add << 4) \| 4)` | `a.t_add = ?t` |
//! | `tm:validFrom` / `tm:validTo` | `v_from` / `v_to` | `((a.v_from << 15) \| 13463)`: a `DATETIME` with offset `Z` | `a.v_from = ?instant` |
//! | `tm:retractKind` | `ret_kind` | `((a.ret_kind << 4) \| 5)` | `a.ret_kind = ?k` |

use tm_core::{ObjectId, Tag};
use tm_ir::vocab;

/// The low 15 bits of a `DATETIME` id with offset `Z`: `(841 << 4) | 7`.
pub const DATETIME_Z_LOW: i64 = (841 << 4) | 7;

/// A virtual predicate.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum VirtualPred {
    /// `sys:subject`.
    Subject,
    /// `sys:predicate`.
    Predicate,
    /// `sys:object`.
    Object,
    /// `tm:txAdded`.
    TxAdded,
    /// `tm:txRetracted`.
    TxRetracted,
    /// `tm:validFrom`.
    ValidFrom,
    /// `tm:validTo`.
    ValidTo,
    /// `tm:retractKind`.
    RetractKind,
}

impl VirtualPred {
    /// The virtual predicate named by `iri`, if it is one.
    // @lat: [[query#Views and Scans#Virtual Predicates]]
    pub fn from_iri(iri: &str) -> Option<VirtualPred> {
        Some(match iri {
            vocab::SYS_SUBJECT => VirtualPred::Subject,
            vocab::SYS_PREDICATE => VirtualPred::Predicate,
            vocab::SYS_OBJECT => VirtualPred::Object,
            vocab::TM_TX_ADDED => VirtualPred::TxAdded,
            vocab::TM_TX_RETRACTED => VirtualPred::TxRetracted,
            vocab::TM_VALID_FROM => VirtualPred::ValidFrom,
            vocab::TM_VALID_TO => VirtualPred::ValidTo,
            vocab::TM_RETRACT_KIND => VirtualPred::RetractKind,
            _ => return None,
        })
    }

    /// The `triple` column it reads.
    pub fn column(self) -> &'static str {
        match self {
            VirtualPred::Subject => "s",
            VirtualPred::Predicate => "p",
            VirtualPred::Object => "o",
            VirtualPred::TxAdded => "t_add",
            VirtualPred::TxRetracted => "t_ret",
            VirtualPred::ValidFrom => "v_from",
            VirtualPred::ValidTo => "v_to",
            VirtualPred::RetractKind => "ret_kind",
        }
    }

    /// True when the column may be NULL (then the virtual triple is absent).
    pub fn nullable(self) -> bool {
        matches!(
            self,
            VirtualPred::TxRetracted
                | VirtualPred::ValidFrom
                | VirtualPred::ValidTo
                | VirtualPred::RetractKind
        )
    }

    /// The SQL expression of the object as an ObjectId, on alias `a`.
    pub fn object_expr(self, a: &str) -> String {
        let c = self.column();
        match self {
            VirtualPred::Subject | VirtualPred::Predicate | VirtualPred::Object => {
                format!("{a}.{c}")
            }
            VirtualPred::TxAdded | VirtualPred::TxRetracted => format!("(({a}.{c} << 4) | 4)"),
            VirtualPred::ValidFrom | VirtualPred::ValidTo => {
                format!("(({a}.{c} << 15) | {DATETIME_Z_LOW})")
            }
            VirtualPred::RetractKind => format!("(({a}.{c} << 4) | 5)"),
        }
    }

    /// `a.col IS NOT NULL` for a nullable column.
    pub fn not_null(self, a: &str) -> Option<String> {
        self.nullable()
            .then(|| format!("{a}.{} IS NOT NULL", self.column()))
    }

    /// The column value a constant object compares with, or `None` when the
    /// constant's kind cannot be this predicate's object (the pattern is empty).
    pub fn object_column_value(self, id: ObjectId) -> Option<i64> {
        let tag = id.tag().ok()?;
        match self {
            VirtualPred::Subject | VirtualPred::Object => Some(id.raw()),
            VirtualPred::Predicate => (tag == Tag::Iri).then_some(id.raw()),
            VirtualPred::TxAdded | VirtualPred::TxRetracted => {
                (tag == Tag::Tx).then_some(id.unsigned_payload() as i64)
            }
            VirtualPred::ValidFrom | VirtualPred::ValidTo => {
                (tag == Tag::DateTime).then_some(id.instant())
            }
            VirtualPred::RetractKind => (tag == Tag::Int).then_some(id.signed_payload()),
        }
    }

    /// `a.col = ?` for a constant object.
    pub fn object_compare(self, a: &str, placeholder: &str) -> String {
        format!("{a}.{} = {placeholder}", self.column())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{codec, Value};

    #[test]
    fn mapping_table() {
        for iri in vocab::VIRTUAL {
            assert!(VirtualPred::from_iri(iri).is_some());
        }
        assert_eq!(VirtualPred::from_iri("urn:tiramemsu:sys:cardinality"), None);
        assert_eq!(
            VirtualPred::TxAdded.object_expr("t0"),
            "((t0.t_add << 4) | 4)"
        );
        assert_eq!(VirtualPred::TxAdded.not_null("t0"), None);
        assert_eq!(
            VirtualPred::ValidTo.not_null("t1").as_deref(),
            Some("t1.v_to IS NOT NULL")
        );
    }

    #[test]
    fn valid_time_object_is_a_utc_datetime() {
        let ms = 1_772_323_200_000i64;
        let expected = match codec::encode(&Value::DateTime { ms, tz: Some(0) }) {
            codec::Encoded::Inline(id) => id.raw(),
            _ => unreachable!(),
        };
        assert_eq!((ms << 15) | DATETIME_Z_LOW, expected);
    }

    #[test]
    fn wrong_kind_constants_are_impossible() {
        let int = ObjectId::from_signed(Tag::Int, 150);
        let tx = ObjectId::from_unsigned(Tag::Tx, 150);
        assert_eq!(VirtualPred::TxAdded.object_column_value(int), None);
        assert_eq!(VirtualPred::TxAdded.object_column_value(tx), Some(150));
        assert_eq!(VirtualPred::RetractKind.object_column_value(int), Some(150));
        assert_eq!(VirtualPred::Predicate.object_column_value(int), None);
        let dt = match codec::encode(&Value::DateTime {
            ms: 7_200_000,
            tz: Some(120),
        }) {
            codec::Encoded::Inline(id) => id,
            _ => unreachable!(),
        };
        assert_eq!(
            VirtualPred::ValidFrom.object_column_value(dt),
            Some(7_200_000)
        );
    }
}
