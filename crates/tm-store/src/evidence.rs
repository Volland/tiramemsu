//! The evidence of a statement: its confidence layer, its confirmations and the
//! authors behind it, read in one view. A port of `tm_core::text::EvidenceReader`,
//! shared by text, vector and hybrid recall and by conflict inspection.

use std::collections::BTreeSet;

use tm_core::text::{TextEvidence, DEFAULT_CONFIDENCE};
use tm_core::vocab;
use tm_core::{Eid, ObjectId, Result, Tag, TxId, Value};

use crate::dict::{self, TermCache};
use crate::read::View;
use crate::store::{Pattern, ReadStore};

/// The evidence reads of one recall or inspection, in one view. A layer
/// predicate that is not interned is `None`.
#[derive(Clone, Copy, Debug)]
pub struct EvidenceReader {
    view: View,
    confidence: Option<ObjectId>,
    /// `sys:confirmedBy`, when interned.
    pub confirmed_by: Option<ObjectId>,
    /// `sys:author`, when interned.
    pub author: Option<ObjectId>,
}

impl EvidenceReader {
    /// The reader of `view`, with `confidence` as the confidence layer
    /// ([`DEFAULT_CONFIDENCE`] when `None`).
    pub fn new(
        store: &dyn ReadStore,
        view: View,
        confidence: Option<ObjectId>,
    ) -> Result<EvidenceReader> {
        let confidence = match confidence {
            Some(c) => Some(c),
            None => dict::encode(store, &Value::iri(DEFAULT_CONFIDENCE))?,
        };
        Ok(EvidenceReader {
            view,
            confidence,
            confirmed_by: dict::encode(store, &Value::iri(vocab::SYS_CONFIRMED_BY))?,
            author: dict::encode(store, &Value::iri(vocab::SYS_AUTHOR))?,
        })
    }

    /// The objects of the selected `(s, p, ?)` statements, in eid order.
    pub fn objects(
        &self,
        store: &dyn ReadStore,
        s: ObjectId,
        p: ObjectId,
    ) -> Result<Vec<ObjectId>> {
        Ok(self
            .view
            .scan_eid(store, Pattern::new(Some(s.raw()), Some(p.raw()), None))?
            .into_iter()
            .map(|r| ObjectId::from_raw(r.o))
            .collect())
    }

    /// The largest numeric object of the selected confidence statements on `me`.
    pub fn confidence(
        &self,
        store: &dyn ReadStore,
        me: ObjectId,
        terms: &TermCache,
        use_cache: bool,
    ) -> Result<Option<f64>> {
        let Some(c) = self.confidence else {
            return Ok(None);
        };
        let mut best: Option<f64> = None;
        for o in self.objects(store, me, c)? {
            let x = match terms.decode(store, o, use_cache)? {
                Value::Int(i) => Some(i as f64),
                Value::Double(d) if !d.is_nan() => Some(d),
                Value::Decimal(s) => s.parse::<f64>().ok(),
                _ => None,
            };
            if let Some(x) = x {
                best = Some(best.map_or(x, |b: f64| b.max(x)));
            }
        }
        Ok(best)
    }

    /// The evidence of statement `eid`, asserted by `t_add` at `added_at`.
    pub fn evidence(
        &self,
        store: &dyn ReadStore,
        eid: Eid,
        t_add: TxId,
        added_at: i64,
        terms: &TermCache,
        use_cache: bool,
    ) -> Result<TextEvidence> {
        let me = eid.oid();
        let mut ev = TextEvidence {
            confidence: self.confidence(store, me, terms, use_cache)?,
            confirmations: 0,
            authors: 0,
            t_add,
            added_at,
        };
        let mut txs = vec![t_add.oid()];
        if let Some(cb) = self.confirmed_by {
            let confirmations = self.objects(store, me, cb)?;
            ev.confirmations = confirmations.len() as u64;
            txs.extend(
                confirmations
                    .into_iter()
                    .filter(|o| o.tag_bits() == Tag::Tx as u8),
            );
        }
        if let Some(author) = self.author {
            let mut seen = BTreeSet::new();
            for t in txs {
                seen.extend(self.objects(store, t, author)?);
            }
            ev.authors = seen.len() as u64;
        }
        Ok(ev)
    }
}
