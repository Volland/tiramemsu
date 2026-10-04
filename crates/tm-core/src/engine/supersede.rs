//! Supersede: correct a statement by cascade-and-replay (design D-12).

use std::collections::{HashMap, HashSet, VecDeque};

use super::schema::Flag;
use super::{reserved, IntoObject, Tx};
use crate::error::{Error, Result};
use crate::exec::SqlValue;
use crate::id::{Eid, ObjectId};
use crate::report::{Patch, RetKind, Valid};
use crate::vocab;

struct Row {
    eid: Eid,
    s: ObjectId,
    p: ObjectId,
    o: ObjectId,
    valid: Valid,
}

impl Tx<'_> {
    fn load_row(&mut self, eid: Eid) -> Result<Option<(Row, bool)>> {
        Ok(self
            .exec
            .first_row(
                "SELECT s, p, o, v_from, v_to, t_ret FROM triple WHERE eid = ?1",
                &[SqlValue::Integer(eid.oid().raw())],
            )?
            .map(|r| {
                let id = |k: usize| ObjectId::from_raw(r[k].as_i64().unwrap_or(0));
                (
                    Row {
                        eid,
                        s: id(0),
                        p: id(1),
                        o: id(2),
                        valid: Valid {
                            from: r[3].as_i64(),
                            to: r[4].as_i64(),
                        },
                    },
                    r[5].is_null(),
                )
            }))
    }

    /// Corrects live statement `root`: retracts its cascade set with kind
    /// `supersede` and replays its structurally retained rows under new eids with references rewired through
    /// the substitution map σ, applying `patch` to the root. Links the new root to
    /// the old one with `sys:supersedes` and returns the new root eid.
    ///
    /// Use it to correct a fact while keeping what was said about it: the layers
    /// (source, confidence) move to the new statement, the old ones stay in history.
    ///
    /// # Errors
    ///
    /// [`Error::NotLive`] when `root` is retracted or unknown, and
    /// [`Error::InvalidPatch`] for an empty interval, no change, or a dropped root
    /// or endpoint. Missing/retracted structural endpoints return [`Error::NotLive`].
    ///
    /// # Example
    ///
    /// ```
    /// use tm_core::{vocab::v, Patch, Store, StoreOptions, TxOptions, Valid, Value};
    /// use tm_rusqlite as host;
    ///
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let path = dir.path().join("db");
    /// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
    /// let report = store.transact(TxOptions::default(), |tx| {
    ///     let fact = tx.assert(Value::iri(v("alice")), Value::iri(v("city")), Value::str("Kyiv"), Valid::ALWAYS)?.eid();
    ///     tx.assert(fact, Value::iri(v("source")), Value::str("chat"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let fact = report.asserted[0];
    /// let report = store.transact(TxOptions::default(), |tx| {
    ///     tx.supersede(fact, Patch::object(Value::str("Lviv")))?;
    ///     Ok(())
    /// })?;
    /// let (old, new) = report.superseded[0];
    /// assert_eq!(old, fact);
    /// assert_ne!(new, fact);
    /// # Ok::<(), tm_core::Error>(())
    /// ```
    // @lat: [[time-model#Operations#Supersede]]
    pub fn supersede(&mut self, root: Eid, patch: Patch) -> Result<Eid> {
        root.oid().check_origin()?;
        let row = match self.load_row(root)? {
            Some((row, true)) => row,
            _ => return Err(Error::NotLive(root)),
        };
        let new_o = match patch.o {
            Some(v) => v.into_object(self)?,
            None => row.o,
        };
        let new_valid = Valid {
            from: patch.v_from.unwrap_or(row.valid.from),
            to: patch.v_to.unwrap_or(row.valid.to),
        };
        if new_valid.check().is_err() {
            return Err(Error::InvalidPatch("empty interval".to_string()));
        }
        if new_o == row.o && new_valid == row.valid {
            return Err(Error::InvalidPatch("no change".to_string()));
        }
        // the new root content is a user write
        self.check_positions(row.s, row.p, new_o)?;
        let p_iri = self.iri_of(row.p)?.unwrap_or_default();
        reserved::check_predicate(&p_iri, row.s.tag()?)?;
        let flag = Flag::from_iri(&p_iri);
        match flag {
            Some(f) => {
                self.validate_flag(f, row.p, row.s, new_o)?;
                self.validate_schema_change(f, row.s, new_o, Some(root))?;
            }
            None => self.check_value_type(row.p, new_o)?,
        }
        self.check_statement_references(row.s, row.p, new_o)?;
        let set = self.cascade_set(root)?;
        let cascade: HashSet<Eid> = set.iter().copied().collect();
        let in_graph = self.sys_lookup(vocab::SYS_IN_GRAPH)?;
        let lineage = self.sys_lookup(vocab::SYS_SUPERSEDES)?;
        let mut rows = Vec::with_capacity(set.len());
        for m in &set {
            let (r, _) = self.load_row(*m)?.expect("cascade member exists");
            rows.push(r);
        }
        // Drop foreign memberships and propagate exclusion through structural
        // dependents. Only retained rows receive fresh identifiers.
        let mut dropped = HashSet::new();
        let mut queue = VecDeque::new();
        let mut dependents: HashMap<Eid, Vec<Eid>> = HashMap::new();
        for r in &rows {
            if Some(r.p) == in_graph && !Eid::from_oid(r.o).is_some_and(|g| cascade.contains(&g)) {
                dropped.insert(r.eid);
                queue.push_back(r.eid);
            }
            if Some(r.p) != lineage {
                for id in [r.s, r.o] {
                    if let Some(target) = Eid::from_oid(id).filter(|e| cascade.contains(e)) {
                        dependents.entry(target).or_default().push(r.eid);
                    }
                }
            }
        }
        while let Some(e) = queue.pop_front() {
            if let Some(ds) = dependents.get(&e) {
                for d in ds {
                    if dropped.insert(*d) {
                        queue.push_back(*d);
                    }
                }
            }
        }
        if dropped.contains(&root) {
            return Err(Error::InvalidPatch(
                "correction would drop its root".to_string(),
            ));
        }
        let sigma: HashMap<Eid, Eid> = set
            .iter()
            .filter(|m| !dropped.contains(m))
            .map(|m| Ok((*m, self.alloc_eid()?)))
            .collect::<Result<_>>()?;
        let new_root = sigma[&root];
        let map = |id: ObjectId| {
            Eid::from_oid(id)
                .and_then(|e| sigma.get(&e))
                .map_or(id, |e| e.oid())
        };
        let mapped_s = map(row.s);
        let mapped_o = map(new_o);
        if mapped_s == new_root.oid() || mapped_o == new_root.oid() {
            return Err(Error::SelfReference(new_root));
        }
        if Some(row.p) != lineage {
            for id in [row.s, new_o] {
                if Eid::from_oid(id).is_some_and(|e| dropped.contains(&e)) {
                    return Err(Error::InvalidPatch(
                        "patched endpoint is dropped".to_string(),
                    ));
                }
            }
        }
        for m in &set {
            self.retract_row(*m, RetKind::Supersede)?;
        }
        // schema pipeline for the new root: unique, then cardinality-one
        self.unique_and_cardinality(
            mapped_s,
            row.p,
            mapped_o,
            new_valid,
            flag.is_some_and(Flag::single_valued),
        )?;
        let copies: HashSet<Eid> = sigma.values().copied().collect();
        for r in &rows {
            if dropped.contains(&r.eid) {
                continue;
            }
            for id in [map(r.s), if r.eid == root { mapped_o } else { map(r.o) }] {
                if let Some(e) = Eid::from_oid(id) {
                    if !copies.contains(&e) {
                        let state = self.live(e)?;
                        if state.is_none() || (Some(r.p) != lineage && state != Some(true)) {
                            return Err(Error::NotLive(e));
                        }
                    }
                }
            }
        }
        for r in rows {
            if dropped.contains(&r.eid) {
                continue;
            }
            let new = sigma[&r.eid];
            let s = map(r.s);
            let (o, valid) = if r.eid == root {
                (mapped_o, new_valid)
            } else {
                (map(r.o), r.valid)
            };
            self.insert_row(new, s, r.p, o, valid)?;
            self.superseded.push((r.eid, new));
        }
        let link_p = self.sys(vocab::SYS_SUPERSEDES)?;
        let link = self.alloc_eid()?;
        self.insert_row(link, new_root.oid(), link_p, root.oid(), Valid::ALWAYS)?;
        Ok(new_root)
    }
}
