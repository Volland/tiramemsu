//! Recursive retraction over subject and object positions.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::supersede::{placeholders, IN_CHUNK};
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
        // Breadth first, one query per frontier chunk. Parents are visited in queue
        // order and each parent's dependents in eid order, so `order` is the same as
        // a node-at-a-time walk.
        let mut level = 0;
        while level < order.len() {
            let frontier: Vec<Eid> = order[level..].to_vec();
            level = order.len();
            for chunk in frontier.chunks(IN_CHUNK) {
                let list = placeholders(chunk.len());
                let sql = format!(
                    "SELECT s, eid FROM triple WHERE s IN ({list}) AND t_ret IS NULL \
                     UNION SELECT o, eid FROM triple WHERE o IN ({list}) AND t_ret IS NULL"
                );
                let params: Vec<SqlValue> = chunk
                    .iter()
                    .map(|e| SqlValue::Integer(e.oid().raw()))
                    .collect();
                let mut children: HashMap<i64, BTreeSet<i64>> = HashMap::new();
                for r in self.exec.rows(&sql, &params)? {
                    if let (Some(parent), Some(eid)) = (r[0].as_i64(), r[1].as_i64()) {
                        children.entry(parent).or_default().insert(eid);
                    }
                }
                for e in chunk {
                    let Some(kids) = children.get(&e.oid().raw()) else {
                        continue;
                    };
                    for &raw in kids {
                        let Some(x) = Eid::from_oid(ObjectId::from_raw(raw)) else {
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
            }
        }
        Ok(order)
    }

    /// Retracts `root` and its cascade set. The root gets `kind`; the rest get
    /// `Cascade` for an explicit retraction, else the same `kind`.
    /// Returns false when `root` is not live.
    pub(crate) fn retract_root(&mut self, root: Eid, kind: RetKind) -> Result<bool> {
        root.oid().check_origin()?;
        if self.live(root)? != Some(true) {
            return Ok(false);
        }
        self.validate_flag_retraction(root)?;
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
