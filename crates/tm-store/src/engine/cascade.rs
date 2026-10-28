//! Recursive retraction over subject and object positions and support links. A port of
//! `tm_core::engine::cascade`.

use std::collections::{BTreeSet, HashSet};

use tm_core::{Eid, Error, ObjectId, Result, RetKind};

use super::Tx;
use crate::store::{Pattern, Scope};

impl Tx<'_> {
    /// The cascade set of `root`: `root` plus every live statement reachable over
    /// subject and object positions and over support links (a live `D` with a live
    /// `(D sys:derivedFrom x)` for a reached `x`), breadth first, each parent's
    /// dependents in eid order. Fails with `CascadeLimitExceeded` above
    /// `max_cascade` statements.
    // @tg: implements:: [[openspec:derived-beliefs#Support cascade]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:retraction-cascade#Cascade over subject and object positions]] {since: "2026-10-28"}
    pub(crate) fn cascade_set(&mut self, root: Eid) -> Result<Vec<Eid>> {
        let limit = self.opts.max_cascade;
        let derived_from = self
            .sys_lookup(tm_core::vocab::SYS_DERIVED_FROM)?
            .map(ObjectId::raw);
        let mut order = vec![root];
        let mut seen: HashSet<Eid> = HashSet::from([root]);
        if order.len() > limit {
            return Err(Error::CascadeLimitExceeded { root, limit });
        }
        let mut i = 0;
        while i < order.len() {
            let parent = order[i].oid().raw();
            i += 1;
            let mut kids: BTreeSet<i64> = BTreeSet::new();
            let mut derived: Vec<i64> = Vec::new();
            for pat in [
                Pattern::new(Some(parent), None, None),
                Pattern::new(None, None, Some(parent)),
            ] {
                for r in self.store.scan(pat, Scope::Live)? {
                    let r = r?;
                    kids.insert(r.eid);
                    // the support rule: a live link (D sys:derivedFrom x) reaches D
                    if Some(r.p) == derived_from && r.o == parent {
                        derived.push(r.s);
                    }
                }
            }
            for d in derived {
                if self.store.statement(d)?.is_some_and(|r| r.live()) {
                    kids.insert(d);
                }
            }
            for raw in kids {
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
