//! ObjectIds: 64-bit values with a 4-bit low tag and a 60-bit payload.
//!
//! `id = (payload << 4) | tag`. `INT`, `DATE` and `DATETIME` use a signed payload
//! (arithmetic shift), every other tag an unsigned one (logical shift).

// @lat: [[data-model#ObjectId]]

use std::fmt;

use crate::error::{Error, Result};

/// The feature name reported when the reserved tag 15 is met.
pub const SEALED_FEATURE: &str = "SEALED (M6)";

/// Width of the counter in the payload of `NODE`, `BNODE`, `STMT` and `TX` ids.
/// The 12 payload bits above it hold the origin.
pub const COUNTER_BITS: u32 = 48;

/// The largest counter value format 1 allocates per kind: 2⁴⁸ − 1.
pub const COUNTER_MAX: u64 = (1 << COUNTER_BITS) - 1;

/// The largest origin a payload can name: 2¹² − 1.
pub const ORIGIN_MAX: u16 = (1 << 12) - 1;

/// The feature name reported when an id with a non-zero origin is met. Format 1
/// writes only origin 0; other origins are reserved for merging files.
pub fn origin_feature(origin: u16) -> String {
    format!("origin {origin} (memory merge)")
}

/// The kind of value an ObjectId holds. Tag 15 (`SEALED`) is reserved for M6.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Tag {
    /// IRI, through the term dictionary.
    Iri = 0,
    /// Anonymous LPG node (inline counter).
    Node = 1,
    /// RDF blank node (inline counter).
    BNode = 2,
    /// Statement eid (inline counter).
    Stmt = 3,
    /// Transaction number `t` (inline).
    Tx = 4,
    /// Signed 60-bit integer (inline).
    Int = 5,
    /// Boolean (inline, 0 or 1).
    Bool = 6,
    /// `xsd:dateTime`: `(epoch_ms << 11) | tz` (inline).
    DateTime = 7,
    /// `xsd:date`: signed days since 1970-01-01 (inline).
    Date = 8,
    /// UTF-8 string of at most 7 bytes (inline).
    ShortStr = 9,
    /// Plain string longer than 7 bytes (dictionary).
    Str = 10,
    /// Language-tagged string (dictionary).
    LangStr = 11,
    /// Any other datatype, or an out-of-range value (dictionary).
    Typed = 12,
    /// `xsd:double` (dictionary, `num` set).
    Double = 13,
    /// `xsd:decimal` (dictionary, `num` set).
    Decimal = 14,
}

impl Tag {
    /// Every non-reserved tag, in numeric order.
    pub const ALL: [Tag; 15] = [
        Tag::Iri,
        Tag::Node,
        Tag::BNode,
        Tag::Stmt,
        Tag::Tx,
        Tag::Int,
        Tag::Bool,
        Tag::DateTime,
        Tag::Date,
        Tag::ShortStr,
        Tag::Str,
        Tag::LangStr,
        Tag::Typed,
        Tag::Double,
        Tag::Decimal,
    ];

    /// True when the payload is a term-dictionary id.
    pub fn is_dictionary(self) -> bool {
        matches!(
            self,
            Tag::Iri | Tag::Str | Tag::LangStr | Tag::Typed | Tag::Double | Tag::Decimal
        )
    }

    /// True when the payload is signed (arithmetic shift).
    pub fn is_signed(self) -> bool {
        matches!(self, Tag::Int | Tag::Date | Tag::DateTime)
    }

    /// True for the kinds allowed in subject position.
    pub fn is_subject(self) -> bool {
        matches!(
            self,
            Tag::Iri | Tag::Node | Tag::BNode | Tag::Stmt | Tag::Tx
        )
    }

    /// True for the allocated kinds whose payload is `origin << 48 | counter`:
    /// `NODE`, `BNODE`, `STMT` and `TX`.
    pub fn is_allocated(self) -> bool {
        matches!(self, Tag::Node | Tag::BNode | Tag::Stmt | Tag::Tx)
    }

    /// Upper-case tag name used in tag IRIs (`sys:INT`, `sys:STMT`, ...).
    pub fn name(self) -> &'static str {
        match self {
            Tag::Iri => "IRI",
            Tag::Node => "NODE",
            Tag::BNode => "BNODE",
            Tag::Stmt => "STMT",
            Tag::Tx => "TX",
            Tag::Int => "INT",
            Tag::Bool => "BOOL",
            Tag::DateTime => "DATETIME",
            Tag::Date => "DATE",
            Tag::ShortStr => "SHORT_STR",
            Tag::Str => "STR",
            Tag::LangStr => "LANG_STR",
            Tag::Typed => "TYPED",
            Tag::Double => "DOUBLE",
            Tag::Decimal => "DECIMAL",
        }
    }

    /// Inverse of [`Tag::name`].
    pub fn from_name(name: &str) -> Option<Tag> {
        Tag::ALL.into_iter().find(|t| t.name() == name)
    }
}

impl TryFrom<u8> for Tag {
    type Error = Error;

    /// Tags 0–14 map to their variant; 15 (`SEALED`) fails with `Unsupported`.
    fn try_from(v: u8) -> Result<Tag> {
        match v {
            0..=14 => Ok(Tag::ALL[v as usize]),
            15 => Err(Error::Unsupported {
                feature: SEALED_FEATURE.to_string(),
            }),
            _ => Err(Error::InvalidTerm {
                position: crate::error::Position::Value,
                reason: format!("tag {v} out of range"),
            }),
        }
    }
}

/// A signed 64-bit value identifier: `(payload << 4) | tag`.
///
/// This is what the `triple` table stores in its `s`, `p` and `o` columns. Small
/// values (integers, booleans, dates, short strings, counters) are held inline in
/// the payload; everything else is a dictionary term id. Ordering is that of the
/// raw integer, so it groups by payload, not by [`Tag`]. Use
/// [`ObjectId::tag`] to learn the kind.
///
/// # Example
///
/// ```
/// use tm_core::{ObjectId, Tag};
///
/// let id = ObjectId::from_signed(Tag::Int, -7);
/// assert_eq!(id.tag().unwrap(), Tag::Int);
/// assert_eq!(id.signed_payload(), -7);
/// assert_eq!(id.raw() & 15, Tag::Int as i64);
/// ```
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(i64);

impl ObjectId {
    /// Wraps a raw stored integer without checking it.
    pub const fn from_raw(raw: i64) -> ObjectId {
        ObjectId(raw)
    }

    /// The raw integer as stored in SQLite.
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// The low 4 bits.
    pub const fn tag_bits(self) -> u8 {
        (self.0 & 15) as u8
    }

    /// The tag; `Unsupported` for the reserved tag 15.
    pub fn tag(self) -> Result<Tag> {
        Tag::try_from(self.tag_bits())
    }

    /// Payload read with an arithmetic shift (for `INT`, `DATE`, `DATETIME`).
    pub const fn signed_payload(self) -> i64 {
        self.0 >> 4
    }

    /// Payload read with a logical shift (for every other tag).
    pub const fn unsigned_payload(self) -> u64 {
        (self.0 as u64) >> 4
    }

    /// Builds an id from a signed payload. The payload must fit in 60 bits.
    pub fn from_signed(tag: Tag, payload: i64) -> ObjectId {
        debug_assert!((-(1i64 << 59)..(1i64 << 59)).contains(&payload));
        ObjectId((payload << 4) | tag as i64)
    }

    /// Builds an id from an unsigned payload. The payload must fit in 60 bits.
    pub fn from_unsigned(tag: Tag, payload: u64) -> ObjectId {
        debug_assert!(payload < (1u64 << 60));
        ObjectId(((payload << 4) | tag as u64) as i64)
    }

    /// The origin of a `NODE`, `BNODE`, `STMT` or `TX` id: the high 12 payload bits
    /// (`payload >> 48`). `None` for every other tag. Format 1 allocates only origin 0.
    ///
    /// ```
    /// use tm_core::{Eid, ObjectId, Tag};
    ///
    /// assert_eq!(Eid::new(42).oid().origin(), Some(0));
    /// assert_eq!(ObjectId::from_unsigned(Tag::Stmt, (1 << 48) | 5).origin(), Some(1));
    /// assert_eq!(ObjectId::from_signed(Tag::Int, 5).origin(), None);
    /// ```
    pub fn origin(self) -> Option<u16> {
        self.allocated()
            .then(|| (self.unsigned_payload() >> COUNTER_BITS) as u16)
    }

    /// The counter of a `NODE`, `BNODE`, `STMT` or `TX` id: the low 48 payload bits.
    /// `None` for every other tag.
    pub fn counter(self) -> Option<u64> {
        self.allocated()
            .then(|| self.unsigned_payload() & COUNTER_MAX)
    }

    fn allocated(self) -> bool {
        Tag::try_from(self.tag_bits()).is_ok_and(Tag::is_allocated)
    }

    /// Fails with `Unsupported` naming the origin when this is a `NODE`, `BNODE`,
    /// `STMT` or `TX` id with a non-zero origin, which format 1 never writes.
    pub fn check_origin(self) -> Result<()> {
        match self.origin() {
            Some(o) if o != 0 => Err(Error::Unsupported {
                feature: origin_feature(o),
            }),
            _ => Ok(()),
        }
    }

    /// The instant of a `DATETIME` id in epoch milliseconds: `id >> 15`.
    pub const fn instant(self) -> i64 {
        self.0 >> 15
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tag() {
            Ok(t) if t.is_signed() => write!(f, "{}:{}", t.name(), self.signed_payload()),
            Ok(t) => write!(f, "{}:{}", t.name(), self.unsigned_payload()),
            Err(_) => write!(f, "SEALED:{}", self.unsigned_payload()),
        }
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// A statement id: an ObjectId whose tag is `STMT`.
///
/// Every statement has one, it is never reused, and it can appear as the subject
/// or object of other statements (that is how layers are built). Displayed as `e<n>`.
///
/// ```
/// use tm_core::{Eid, Tag};
///
/// let e = Eid::new(42);
/// assert_eq!(e.to_string(), "e42");
/// assert_eq!(e.oid().tag().unwrap(), Tag::Stmt);
/// assert_eq!(Eid::from_oid(e.oid()), Some(e));
/// ```
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Eid(ObjectId);

impl Eid {
    /// The eid with statement number `n`.
    pub fn new(n: u64) -> Eid {
        Eid(ObjectId::from_unsigned(Tag::Stmt, n))
    }

    /// Wraps an ObjectId if its tag is `STMT`.
    pub fn from_oid(id: ObjectId) -> Option<Eid> {
        (id.tag_bits() == Tag::Stmt as u8).then_some(Eid(id))
    }

    /// The ObjectId of this eid.
    pub const fn oid(self) -> ObjectId {
        self.0
    }

    /// The statement number (payload).
    pub const fn n(self) -> u64 {
        self.0.unsigned_payload()
    }
}

impl fmt::Debug for Eid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.n())
    }
}

impl fmt::Display for Eid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.n())
    }
}

impl From<Eid> for ObjectId {
    fn from(e: Eid) -> ObjectId {
        e.0
    }
}

/// A transaction number `t`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TxId(pub u64);

impl TxId {
    /// The `TX` ObjectId of this transaction.
    pub fn oid(self) -> ObjectId {
        ObjectId::from_unsigned(Tag::Tx, self.0)
    }

    /// The transaction number.
    pub const fn t(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tx{}", self.0)
    }
}

impl From<TxId> for ObjectId {
    fn from(t: TxId) -> ObjectId {
        t.oid()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_and_payload_extraction() {
        let id = ObjectId::from_signed(Tag::Int, 5);
        assert_eq!(id.raw(), 85);
        assert_eq!(id.raw() & 15, 5);
        assert_eq!(id.tag().unwrap(), Tag::Int);
        assert_eq!(id.signed_payload(), 5);
    }

    #[test]
    fn statement_and_transaction_ids_are_inline() {
        assert_eq!(Eid::new(42).oid().raw(), (42 << 4) | 3);
        assert_eq!(TxId(7).oid().raw(), (7 << 4) | 4);
        assert!(!Tag::Stmt.is_dictionary());
        assert!(!Tag::Tx.is_dictionary());
    }

    #[test]
    fn signed_and_unsigned_payloads() {
        let neg = ObjectId::from_signed(Tag::Int, -1);
        assert_eq!(neg.signed_payload(), -1);
        assert_eq!(neg.tag().unwrap(), Tag::Int);
        let big = ObjectId::from_unsigned(Tag::ShortStr, (1u64 << 60) - 1);
        assert_eq!(big.unsigned_payload(), (1u64 << 60) - 1);
        assert!(big.raw() < 0);
        assert_eq!(big.tag().unwrap(), Tag::ShortStr);
    }

    #[test]
    fn tags_map_to_numbers() {
        for (i, t) in Tag::ALL.iter().enumerate() {
            assert_eq!(*t as u8, i as u8);
            assert_eq!(Tag::try_from(i as u8).unwrap(), *t);
            assert_eq!(Tag::from_name(t.name()), Some(*t));
        }
    }

    // @lat: [[tests#ObjectId#Origin And Counter Split]]
    #[test]
    fn origin_and_counter_split() {
        // local ids: origin 0, the counter is the whole payload, the id is unchanged
        let e = Eid::new(42).oid();
        assert_eq!((e.origin(), e.counter()), (Some(0), Some(42)));
        assert_eq!(e.raw(), (42 << 4) | 3);
        assert!(e.check_origin().is_ok());
        // the bounds of the counter
        for tag in [Tag::Node, Tag::BNode, Tag::Stmt, Tag::Tx] {
            let last = ObjectId::from_unsigned(tag, COUNTER_MAX);
            assert_eq!(
                (last.origin(), last.counter()),
                (Some(0), Some(COUNTER_MAX))
            );
            assert!(last.check_origin().is_ok());
            let first_foreign = ObjectId::from_unsigned(tag, COUNTER_MAX + 1);
            assert_eq!(
                (first_foreign.origin(), first_foreign.counter()),
                (Some(1), Some(0))
            );
            let top = ObjectId::from_unsigned(tag, (1u64 << 60) - 1);
            assert_eq!(
                (top.origin(), top.counter()),
                (Some(ORIGIN_MAX), Some(COUNTER_MAX))
            );
            assert_eq!(top.tag().unwrap(), tag);
        }
        let foreign = ObjectId::from_unsigned(Tag::Stmt, (1 << 48) | 5);
        assert_eq!((foreign.origin(), foreign.counter()), (Some(1), Some(5)));
        match foreign.check_origin() {
            Err(Error::Unsupported { feature }) => assert!(feature.contains("origin 1")),
            other => panic!("unexpected {other:?}"),
        }
        // other tags have no origin, whatever their payload
        for id in [
            ObjectId::from_signed(Tag::Int, -1),
            ObjectId::from_unsigned(Tag::Iri, COUNTER_MAX + 1),
            ObjectId::from_unsigned(Tag::ShortStr, (1u64 << 60) - 1),
            ObjectId::from_raw((3 << 4) | 15),
        ] {
            assert_eq!((id.origin(), id.counter()), (None, None));
            assert!(id.check_origin().is_ok());
        }
    }

    #[test]
    fn sealed_tag_is_unsupported() {
        match Tag::try_from(15) {
            Err(Error::Unsupported { feature }) => {
                assert!(feature.contains("SEALED") && feature.contains("M6"))
            }
            other => panic!("unexpected {other:?}"),
        }
        match ObjectId::from_raw((3 << 4) | 15).tag() {
            Err(Error::Unsupported { .. }) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
