//! Recursive retraction over subject and object positions and support links.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::supersede::{placeholders, IN_CHUNK};
use super::Tx;
use crate::error::{Error, Result};
use crate::exec::SqlValue;
use crate::id::{Eid, ObjectId};
use crate::report::RetKind;
use crate::vocab;

impl Tx<'_> {
    /// The cascade set of `root`: `root` plus every live statement reachable over
    /// subject and object positions and over support links (a live `D` with a live
    /// `(D sys:derivedFrom x)` for a reached `x`), in BFS order. Fails with
    /// `CascadeLimitExceeded` when it would hold more than `max_cascade` statements.
    // @lat: [[time-model#Cascade]]
    // @tg: implements:: [[openspec:derived-beliefs#Support cascade]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:retraction-cascade#Cascade over subject and object positions]] {since: "2026-10-28"}
    pub(crate) fn cascade_set(&mut self, root: Eid) -> Result<Vec<Eid>> {
        let limit = self.opts.max_cascade;
        let derived_from = self.sys_lookup(vocab::SYS_DERIVED_FROM)?;
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
                let mut sql = format!(
                    "SELECT s, eid FROM triple WHERE s IN ({list}) AND t_ret IS NULL \
                     UNION SELECT o, eid FROM triple WHERE o IN ({list}) AND t_ret IS NULL"
                );
                let mut params: Vec<SqlValue> = chunk
                    .iter()
                    .map(|e| SqlValue::Integer(e.oid().raw()))
                    .collect();
                if let Some(df) = derived_from {
                    // the support rule: a live link (D sys:derivedFrom x) reaches D
                    sql.push_str(&format!(
                        " UNION SELECT l.o, l.s FROM triple l JOIN triple d ON d.eid = l.s \
                         WHERE l.o IN ({list}) AND l.p = ?{} AND l.t_ret IS NULL AND d.t_ret IS NULL",
                        chunk.len() + 1
                    ));
                    params.push(SqlValue::Integer(df.raw()));
                }
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
