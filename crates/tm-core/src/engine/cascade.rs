//! Recursive retraction over subject and object positions.

use std::collections::HashSet;

use super::Tx;
use crate::error::{Error, Result};
use crate::exec::SqlValue;
use crate::id::{Eid, ObjectId};
use crate::report::RetKind;

impl Tx<'_> {
    /// The cascade set of `root`: `root` plus every live statement reachable over
    /// subject and object positions, in BFS order. Fails with
    /// `CascadeLimitExceeded` when it would hold more than `max_cascade` statements.
    // @lat: [[time-model#Cascade]]
    pub(crate) fn cascade_set(&mut self, root: Eid) -> Result<Vec<Eid>> {
        let limit = self.opts.max_cascade;
        let mut order = vec![root];
        let mut seen: HashSet<Eid> = HashSet::from([root]);
        if order.len() > limit {
            return Err(Error::CascadeLimitExceeded { root, limit });
        }
        let mut i = 0;
        while i < order.len() {
            let e = order[i];
            i += 1;
            let rows = self.exec.rows(
                "SELECT eid FROM triple WHERE s = ?1 AND t_ret IS NULL \
                 UNION SELECT eid FROM triple WHERE o = ?1 AND t_ret IS NULL ORDER BY eid",
                &[SqlValue::Integer(e.oid().raw())],
            )?;
            for r in rows {
                let Some(x) = r[0]
                    .as_i64()
                    .and_then(|r| Eid::from_oid(ObjectId::from_raw(r)))
                else {
                    continue;
                };
                if seen.insert(x) {
                    order.push(x);
                    if order.len() > limit {
                        return Err(Error::CascadeLimitExceeded { root, limit });
                    }
                }
            }
        }
        Ok(order)
    }

    /// Retracts `root` and its cascade set. The root gets `kind`; the rest get
    /// `Cascade` for an explicit retraction, else the same `kind`.
    /// Returns false when `root` is not live.
    pub(crate) fn retract_root(&mut self, root: Eid, kind: RetKind) -> Result<bool> {
        if self.live(root)? != Some(true) {
            return Ok(false);
        }
        let set = self.cascade_set(root)?;
        for (i, e) in set.into_iter().enumerate() {
            let k = if i == 0 || kind != RetKind::Explicit {
                kind
            } else {
                RetKind::Cascade
            };
            self.retract_row(e, k)?;
        }
        Ok(true)
    }
}
