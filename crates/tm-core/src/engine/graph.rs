//! Named graphs: membership statements `(eid sys:inGraph graph)` (spec `named-graphs`).
//!
//! A graph is a node. A membership is an ordinary statement with its own eid,
//! lifetime and valid time, so it is bitemporal and carries layers. Only these
//! methods (and the SPARQL `GRAPH` blocks built on them) write `sys:inGraph`.

use super::ops::Writer;
use super::{IntoObject, Tx};
use crate::error::{Error, Result};
use crate::exec::SqlValue;
use crate::id::{Eid, ObjectId, Tag};
use crate::report::{AssertOpts, Asserted, RetKind, Valid};
use crate::vocab;

impl Tx<'_> {
    /// Validates a graph name: an `IRI`, `NODE` or `BNODE` id, otherwise
    /// `InvalidGraphName`.
    // @lat: [[data-model#Named Graphs]]
    fn graph_id(&mut self, graph: impl IntoObject) -> Result<ObjectId> {
        let g = graph.into_object(self)?;
        match g.tag()? {
            Tag::Iri | Tag::Node | Tag::BNode => {
                self.check_known(g, crate::error::Position::Object)?;
                Ok(g)
            }
            _ => {
                let term = self.render_graph(g);
                Err(Error::InvalidGraphName { term })
            }
        }
    }

    /// The rendered form of a graph id, for error messages.
    pub(crate) fn render_graph(&mut self, g: ObjectId) -> String {
        self.decode(g)
            .map(|v| v.to_string())
            .unwrap_or_else(|_| format!("{g:?}"))
    }

    /// Adds live statement `eid` to `graph`: asserts `(eid sys:inGraph graph)`
    /// idempotently over `opts.valid` and returns the membership eid and whether it
    /// is new. Fails with `InvalidGraphName` for a non-node graph, `NotLive` for a
    /// retracted statement and `ReservedNamespace` for a statement whose predicate
    /// is in `sys:` (schema and bookkeeping statements belong to no user graph).
    pub fn add_to_graph(
        &mut self,
        eid: Eid,
        graph: impl IntoObject,
        opts: AssertOpts,
    ) -> Result<(Eid, bool)> {
        eid.oid().check_origin()?;
        let g = self.graph_id(graph)?;
        let Some(row) = self.exec.first_row(
            "SELECT p, t_ret FROM triple WHERE eid = ?1",
            &[SqlValue::Integer(eid.oid().raw())],
        )?
        else {
            return Err(Error::NotLive(eid));
        };
        if !row[1].is_null() {
            return Err(Error::NotLive(eid));
        }
        let p = ObjectId::from_raw(row[0].as_i64().unwrap_or(0));
        let p_iri = self.iri_of(p)?.unwrap_or_default();
        if p_iri.starts_with(vocab::SYS) || p_iri.starts_with(vocab::TM) {
            return Err(Error::ReservedNamespace(p_iri));
        }
        let ig = self.sys(vocab::SYS_IN_GRAPH)?;
        self.in_graph = Some(Some(ig));
        let r = self.write(eid.oid(), ig, g, opts.valid, true, Writer::Engine)?;
        if r.is_new() {
            // a membership is listed as a membership, not as an asserted statement
            self.asserted.retain(|e| *e != r.eid());
            self.memberships.push(r.eid());
        }
        Ok((r.eid(), r.is_new()))
    }

    /// Removes statement `eid` from `graph`: retracts its live memberships in
    /// `graph`. The statement stays live. Returns whether one was live.
    pub fn remove_from_graph(&mut self, eid: Eid, graph: impl IntoObject) -> Result<bool> {
        eid.oid().check_origin()?;
        let g = self.graph_id(graph)?;
        let Some(ig) = self.sys_lookup(vocab::SYS_IN_GRAPH)? else {
            return Ok(false);
        };
        let ms = self.live_memberships(Some(eid), ig, g)?;
        for m in &ms {
            self.retract_root(*m, RetKind::Explicit)?;
        }
        Ok(!ms.is_empty())
    }

    /// Retracts every live membership in `graph` and returns the membership eids.
    /// Member statements stay live.
    pub fn clear_graph(&mut self, graph: impl IntoObject) -> Result<Vec<Eid>> {
        let g = self.graph_id(graph)?;
        let Some(ig) = self.sys_lookup(vocab::SYS_IN_GRAPH)? else {
            return Ok(Vec::new());
        };
        let ms = self.live_memberships(None, ig, g)?;
        for m in &ms {
            self.retract_root(*m, RetKind::Explicit)?;
        }
        Ok(ms)
    }

    /// Declares `graph`: asserts `(graph rdf:type sys:Graph)` idempotently. The
    /// result says whether the declaration was new.
    pub fn create_graph(&mut self, graph: impl IntoObject) -> Result<Asserted> {
        let g = self.graph_id(graph)?;
        let ty = self.sys(vocab::RDF_TYPE)?;
        let cls = self.sys(vocab::SYS_GRAPH)?;
        let r = self.write(g, ty, cls, Valid::ALWAYS, true, Writer::User)?;
        if let Asserted::Existing(e) = r {
            self.existing.push(e);
        }
        Ok(r)
    }

    /// Clears `graph` and retracts its `sys:Graph` declaration. Other triples about
    /// the graph node stay. Returns every retracted root eid (memberships first).
    pub fn drop_graph(&mut self, graph: impl IntoObject) -> Result<Vec<Eid>> {
        let g = self.graph_id(graph)?;
        let mut out = self.clear_graph(g)?;
        if let (Some(ty), Some(cls)) = (
            self.sys_lookup(vocab::RDF_TYPE)?,
            self.sys_lookup(vocab::SYS_GRAPH)?,
        ) {
            out.extend(self.retract_matching(Some(g), Some(ty), Some(cls))?);
        }
        Ok(out)
    }

    /// Whether `graph` has a live declaration.
    pub fn graph_declared(&mut self, graph: impl IntoObject) -> Result<bool> {
        let g = self.graph_id(graph)?;
        let (Some(ty), Some(cls)) = (
            self.sys_lookup(vocab::RDF_TYPE)?,
            self.sys_lookup(vocab::SYS_GRAPH)?,
        ) else {
            return Ok(false);
        };
        Ok(self
            .exec
            .query_i64(
                "SELECT eid FROM triple WHERE s = ?1 AND p = ?2 AND o = ?3 AND t_ret IS NULL LIMIT 1",
                &[
                    SqlValue::Integer(g.raw()),
                    SqlValue::Integer(ty.raw()),
                    SqlValue::Integer(cls.raw()),
                ],
            )?
            .is_some())
    }

    /// Whether `graph` has a live membership.
    pub fn graph_has_members(&mut self, graph: impl IntoObject) -> Result<bool> {
        let g = self.graph_id(graph)?;
        let Some(ig) = self.sys_lookup(vocab::SYS_IN_GRAPH)? else {
            return Ok(false);
        };
        Ok(!self.live_memberships(None, ig, g)?.is_empty())
    }

    /// Every graph with a live membership or a live declaration, ascending.
    pub fn live_graphs(&mut self) -> Result<Vec<ObjectId>> {
        self.exec_graphs()
    }

    fn exec_graphs(&mut self) -> Result<Vec<ObjectId>> {
        crate::read::graphs(&mut *self.exec, &crate::view::ViewSpec::NOW)
    }

    /// The live memberships `(m, e, sys:inGraph, g)`, optionally for one `e`.
    fn live_memberships(
        &mut self,
        eid: Option<Eid>,
        ig: ObjectId,
        g: ObjectId,
    ) -> Result<Vec<Eid>> {
        let mut sql =
            String::from("SELECT eid FROM triple WHERE p = ?1 AND o = ?2 AND t_ret IS NULL");
        let mut params = vec![SqlValue::Integer(ig.raw()), SqlValue::Integer(g.raw())];
        if let Some(e) = eid {
            sql.push_str(" AND s = ?3");
            params.push(SqlValue::Integer(e.oid().raw()));
        }
        sql.push_str(" ORDER BY eid");
        Ok(self
            .exec
            .rows(&sql, &params)?
            .into_iter()
            .filter_map(|r| r[0].as_i64())
            .filter_map(|r| Eid::from_oid(ObjectId::from_raw(r)))
            .collect())
    }
}
