//! Supersede: correct a statement by cascade-and-replay. A port of
//! `tm_core::engine::supersede`.

use std::collections::{HashMap, HashSet, VecDeque};

use tm_core::engine::reserved;
use tm_core::vocab;
use tm_core::{Eid, Error, ObjectId, Patch, Result, RetKind, Valid};

use super::schema::Flag;
use super::{IntoObject, Tx};
use crate::store::StmtRow;

struct Row {
    eid: Eid,
    s: ObjectId,
    p: ObjectId,
    o: ObjectId,
    valid: Valid,
}

fn row_of(eid: Eid, r: &StmtRow) -> Row {
    Row {
        eid,
        s: ObjectId::from_raw(r.s),
        p: ObjectId::from_raw(r.p),
        o: ObjectId::from_raw(r.o),
        valid: Valid {
            from: r.v_from,
            to: r.v_to,
        },
    }
}

impl Tx<'_> {
    fn load_row(&mut self, eid: Eid) -> Result<Option<(Row, bool)>> {
        Ok(self
            .store
            .statement(eid.oid().raw())?
            .map(|r| (row_of(eid, &r), r.live())))
    }

    /// The rows of `set`, in its order. A missing row is [`Error::NotLive`].
    fn load_rows(&mut self, set: &[Eid]) -> Result<Vec<Row>> {
        set.iter()
            .map(|e| {
                self.store
                    .statement(e.oid().raw())?
                    .map(|r| row_of(*e, &r))
                    .ok_or(Error::NotLive(*e))
            })
            .collect()
    }

    /// Corrects live statement `root`: retracts its cascade set with kind
    /// `supersede` and replays its structurally retained rows under new eids with references rewired through
    /// the substitution map σ, applying `patch` to the root. Links the new root to
    /// the old one with `sys:supersedes` and returns the new root eid.
    ///
    /// Use it to correct a fact while keeping what was said about it: the layers
    /// (source, confidence) move to the new statement, the old ones stay in history.
    /// Beliefs derived from the corrected statement ([`Tx::derive`]) are retracted
    /// and not replayed, since they were concluded from the old content; a derived
    /// root keeps its support links, so a patch of its valid time widens it.
    ///
    /// # Errors
    ///
    /// [`Error::NotLive`] when `root` is retracted or unknown, and
    /// [`Error::InvalidPatch`] for an empty interval, no change, or a dropped root
    /// or endpoint. Missing/retracted structural endpoints return [`Error::NotLive`].
    ///
    // @tg: implements:: [[openspec:derived-beliefs#Supersede and derived statements]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:derived-beliefs#Widening a derivation]] {since: "2026-10-28"}
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
        let derived_from = self.sys_lookup(vocab::SYS_DERIVED_FROM)?;
        let rows = self.load_rows(&set)?;
        // Drop foreign memberships and the derived statements reached through a
        // support link (other than the root: they were concluded from the old
        // content), and propagate exclusion through structural dependents. Only
        // retained rows receive fresh identifiers.
        let mut dropped = HashSet::new();
        let mut queue = VecDeque::new();
        let mut dependents: HashMap<Eid, Vec<Eid>> = HashMap::new();
        for r in &rows {
            if Some(r.p) == in_graph && !Eid::from_oid(r.o).is_some_and(|g| cascade.contains(&g)) {
                dropped.insert(r.eid);
                queue.push_back(r.eid);
            }
            if Some(r.p) == derived_from && Eid::from_oid(r.o).is_some_and(|e| cascade.contains(&e))
            {
                if let Some(d) = Eid::from_oid(r.s).filter(|d| *d != root && cascade.contains(d)) {
                    if dropped.insert(d) {
                        queue.push_back(d);
                    }
                }
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
