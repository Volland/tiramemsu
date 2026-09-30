//! View resolution: `AsOf(Instant(ms))` becomes the largest `t` whose instant is
//! `≤ ms`, read on the executing connection so it shares the query's snapshot. A
//! point before the first transaction resolves to `None` (the pattern is empty).

use tm_core::{Executor, Result, SqlValue, TimeRef, TxSel};
use tm_ir::View;

use crate::scan::{ResolvedTx, ResolvedView};

/// The transaction number an instant resolves to (0 = before the first tx).
pub fn instant_to_t(exec: &mut dyn Executor, ms: i64) -> Result<u64> {
    Ok(exec
        .query_i64(
            "SELECT coalesce(max(t), 0) FROM tx WHERE instant <= ?1",
            &[SqlValue::Integer(ms)],
        )?
        .unwrap_or(0)
        .max(0) as u64)
}

/// Resolves a pattern view; `None` when it selects a point before transaction 1.
pub fn resolve(exec: &mut dyn Executor, v: &View) -> Result<Option<ResolvedView>> {
    let tx = match v.tx {
        TxSel::Now => ResolvedTx::Now,
        TxSel::History => ResolvedTx::History,
        TxSel::AsOf(TimeRef::Tx(t)) => {
            if t == 0 {
                return Ok(None);
            }
            ResolvedTx::AsOf(t)
        }
        TxSel::AsOf(TimeRef::Instant(ms)) => match instant_to_t(exec, ms)? {
            0 => return Ok(None),
            t => ResolvedTx::AsOf(t),
        },
    };
    Ok(Some(ResolvedView { tx, valid: v.valid }))
}
