//! Lossless copy of a whole database between stores (`tiramemsu::convert`).
//!
//! The copy replays the history in transaction order: every term, then for each
//! transaction its row, the statements it added and the retractions it made, so
//! that a destination which checks transaction dates (the SQLite triggers of
//! format 4) accepts every write. Counters, burned ids included, come last. The
//! destination's derived indexes are rebuilt by the caller.

use std::collections::BTreeMap;

use tm_core::{Error, Result, RetKind};

use crate::engine::load_counters;
use crate::store::{ReadStore, StmtRow, WriteStore};

/// What a copy wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyReport {
    /// Dictionary terms.
    pub terms: u64,
    /// Transactions.
    pub transactions: u64,
    /// Statements.
    pub statements: u64,
    /// Retractions.
    pub retractions: u64,
}

/// Copies every row of `src` into the empty `dst`.
pub fn copy(src: &dyn ReadStore, dst: &mut dyn WriteStore) -> Result<CopyReport> {
    let c = load_counters(src)?;
    let mut rep = CopyReport::default();
    for id in 1..c.next_term {
        if let Some(t) = src.term(id)? {
            dst.insert_term(id, &t)?;
            rep.terms += 1;
        }
    }
    let mut adds: BTreeMap<i64, Vec<StmtRow>> = BTreeMap::new();
    for r in src.added_since(0)? {
        let r = r?;
        adds.entry(r.t_add).or_default().push(r);
    }
    let mut rets: BTreeMap<i64, Vec<StmtRow>> = BTreeMap::new();
    for r in src.retracted_since(0)? {
        let r = r?;
        if let Some(t) = r.t_ret {
            rets.entry(t).or_default().push(r);
        }
    }
    for t in 1..=c.last_t {
        let instant = src
            .tx_instant(t)?
            .ok_or_else(|| Error::storage(format!("transaction {t} has no row")))?;
        dst.insert_tx(t, instant)?;
        rep.transactions += 1;
        for r in adds.remove(&t).unwrap_or_default() {
            dst.insert_statement(&StmtRow {
                t_ret: None,
                ret_kind: None,
                ..r
            })?;
            rep.statements += 1;
        }
        for r in rets.remove(&t).unwrap_or_default() {
            let kind = r.ret_kind.and_then(RetKind::from_i64).ok_or_else(|| {
                Error::storage(format!("statement {} has no retraction kind", r.eid))
            })?;
            dst.retract(r.eid, t, kind)?;
            rep.retractions += 1;
        }
        dst.set_meta("last_t", t)?;
    }
    for p in src.pred_multi_all()? {
        dst.add_pred_multi(p)?;
    }
    for (s, k, v) in src.volatile_all()? {
        dst.put_volatile(s, k, v)?;
    }
    for e in src.embeddings_all()? {
        dst.insert_embedding(&e)?;
    }
    for (k, v) in [
        ("next_term", c.next_term),
        ("next_node", c.next_node),
        ("next_bnode", c.next_bnode),
        ("next_stmt", c.next_stmt),
        ("last_t", c.last_t),
        ("last_instant", c.last_instant),
        ("multi_version", c.multi_version),
    ] {
        dst.set_meta(k, v)?;
    }
    Ok(rep)
}
