//! Reads over the storage traits: views, triple lookup, values, the event log,
//! dependents, graphs and bundles. A port of `tm_core::read` with the time
//! predicates of `tm_core::view::scan_predicates` as row filters.

use std::collections::{BTreeSet, HashSet};

use tm_core::event::{Event, Op};
use tm_core::{
    BundleSource, Eid, ObjectId, Result, RetKind, TimeRef, Triple, TxId, TxSel, Valid, ValidSel,
    Value, ViewSpec,
};

use crate::dict::{self, TermCache};
use crate::store::{Pattern, ReadStore, Scope, StmtRow};

/// A [`ViewSpec`] with its instant resolved to a transaction number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    /// Transaction-time selection.
    pub tx: ViewTx,
    /// Valid-time instant, if filtered.
    pub valid_at: Option<i64>,
}

/// Transaction-time selection of a resolved view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewTx {
    /// Live statements.
    Now,
    /// Statements believed at transaction `t`.
    AsOf(i64),
    /// Every statement.
    History,
}

impl View {
    /// The now view.
    pub const NOW: View = View {
        tx: ViewTx::Now,
        valid_at: None,
    };

    /// Resolves `spec` in `store` (an instant becomes the last transaction at or
    /// before it, 0 when none).
    pub fn resolve(store: &dyn ReadStore, spec: &ViewSpec) -> Result<View> {
        let tx = match spec.tx {
            TxSel::Now => ViewTx::Now,
            TxSel::History => ViewTx::History,
            TxSel::AsOf(TimeRef::Tx(t)) => ViewTx::AsOf(t as i64),
            TxSel::AsOf(TimeRef::Instant(ms)) => ViewTx::AsOf(store.tx_at_or_before(ms)?),
        };
        let valid_at = match spec.valid {
            ValidSel::Unfiltered => None,
            ValidSel::At(d) => Some(d),
        };
        Ok(View { tx, valid_at })
    }

    /// The scan scope that holds every statement this view can select.
    pub fn scope(&self) -> Scope {
        match self.tx {
            ViewTx::Now => Scope::Live,
            _ => Scope::All,
        }
    }

    /// True when the view selects `r` (the predicates of `scan_predicates`).
    pub fn selects(&self, r: &StmtRow) -> bool {
        let tx_ok = match self.tx {
            ViewTx::Now => r.t_ret.is_none(),
            ViewTx::AsOf(t) => r.t_add <= t && r.t_ret.is_none_or(|x| x > t),
            ViewTx::History => true,
        };
        tx_ok
            && self
                .valid_at
                .is_none_or(|d| r.v_from.is_none_or(|f| f <= d) && r.v_to.is_none_or(|t| t > d))
    }

    /// True when rows of this view hide their retraction (as-of views).
    pub fn masks_retraction(&self) -> bool {
        matches!(self.tx, ViewTx::AsOf(_))
    }

    /// The statements of `pat` selected by this view, in the backend's order.
    pub fn scan<'s>(
        &self,
        store: &'s dyn ReadStore,
        pat: Pattern,
    ) -> Result<impl Iterator<Item = Result<StmtRow>> + 's> {
        let me = *self;
        Ok(store
            .scan(pat, self.scope())?
            .filter(move |r| r.as_ref().map_or(true, |r| me.selects(r))))
    }

    /// The statements of `pat` selected by this view, in eid order.
    pub fn scan_eid(&self, store: &dyn ReadStore, pat: Pattern) -> Result<Vec<StmtRow>> {
        let mut rows: Vec<StmtRow> = store
            .scan_eid(pat, self.scope())?
            .into_iter()
            .filter(|r| self.selects(r))
            .collect();
        rows.sort_unstable_by_key(|r| r.eid);
        Ok(rows)
    }
}

/// Converts a stored row to a [`Triple`], hiding the retraction for as-of views.
pub fn to_triple(r: &StmtRow, mask_retraction: bool) -> Triple {
    Triple {
        eid: Eid::from_oid(ObjectId::from_raw(r.eid)).unwrap_or(Eid::new(0)),
        s: ObjectId::from_raw(r.s),
        p: ObjectId::from_raw(r.p),
        o: ObjectId::from_raw(r.o),
        t_add: TxId(r.t_add as u64),
        t_ret: if mask_retraction {
            None
        } else {
            r.t_ret.map(|t| TxId(t as u64))
        },
        v_from: r.v_from,
        v_to: r.v_to,
        ret_kind: if mask_retraction {
            None
        } else {
            r.ret_kind.and_then(RetKind::from_i64)
        },
    }
}

fn pattern(s: Option<ObjectId>, p: Option<ObjectId>, o: Option<ObjectId>) -> Pattern {
    Pattern::new(
        s.map(ObjectId::raw),
        p.map(ObjectId::raw),
        o.map(ObjectId::raw),
    )
}

/// Every statement selected by `spec` that matches the bound positions, ordered
/// by eid. As-of rows report `t_ret` and `ret_kind` as `None`.
pub fn triples(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    s: Option<ObjectId>,
    p: Option<ObjectId>,
    o: Option<ObjectId>,
) -> Result<Vec<Triple>> {
    let view = View::resolve(store, spec)?;
    let mask = view.masks_retraction();
    Ok(view
        .scan_eid(store, pattern(s, p, o))?
        .iter()
        .map(|r| to_triple(r, mask))
        .collect())
}

/// The values of `(s, key)`: objects of the statements selected by the view, or,
/// only for a now view and only when there is no such statement, the volatile value.
pub fn values(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    s: ObjectId,
    key: ObjectId,
) -> Result<Vec<ObjectId>> {
    let objs: Vec<ObjectId> = triples(store, spec, Some(s), Some(key), None)?
        .into_iter()
        .map(|t| t.o)
        .collect();
    if !objs.is_empty() || spec.tx != TxSel::Now {
        return Ok(objs);
    }
    Ok(store
        .volatile(s.raw(), key.raw())?
        .map(|v| ObjectId::from_raw(v.value))
        .into_iter()
        .collect())
}

/// Every event with `t > since`, ordered by time, asserts before retracts, then eid.
pub fn events_since(store: &dyn ReadStore, since: u64) -> Result<Vec<Event>> {
    let since = since as i64;
    let eid = |r: &StmtRow| Eid::from_oid(ObjectId::from_raw(r.eid)).unwrap_or(Eid::new(0));
    let mut out = Vec::new();
    for r in store.added_since(since)? {
        let r = r?;
        out.push(Event {
            t: TxId(r.t_add as u64),
            eid: eid(&r),
            op: Op::Assert,
            kind: None,
        });
    }
    for r in store.retracted_since(since)? {
        let r = r?;
        let Some(t) = r.t_ret else { continue };
        out.push(Event {
            t: TxId(t as u64),
            eid: eid(&r),
            op: Op::Retract,
            kind: r.ret_kind.and_then(RetKind::from_i64),
        });
    }
    let op = |o: Op| match o {
        Op::Assert => 0,
        Op::Retract => 1,
    };
    out.sort_by_key(|e| (e.t, op(e.op), e.eid));
    Ok(out)
}

/// The last committed transaction number.
pub fn last_t(store: &dyn ReadStore) -> Result<u64> {
    Ok(store.meta("last_t")?.unwrap_or(0) as u64)
}

/// The instant of transaction `t`, if it exists.
pub fn tx_instant(store: &dyn ReadStore, t: u64) -> Result<Option<i64>> {
    store.tx_instant(t as i64)
}

/// True when statement `eid` is selected by the view.
pub fn visible(store: &dyn ReadStore, view: &View, eid: Eid) -> Result<bool> {
    Ok(store
        .statement(eid.oid().raw())?
        .is_some_and(|r| view.selects(&r)))
}

/// The statements that stand on `root` in `spec` (see `tm_core::read::dependents`):
/// `root` first, then breadth-first every selected statement whose subject or
/// object was reached, each expansion in ascending eid order.
pub fn dependents(store: &dyn ReadStore, spec: &ViewSpec, root: Eid) -> Result<Vec<Eid>> {
    let view = View::resolve(store, spec)?;
    dependents_in(store, &view, root)
}

fn dependents_in(store: &dyn ReadStore, view: &View, root: Eid) -> Result<Vec<Eid>> {
    if !visible(store, view, root)? {
        return Ok(Vec::new());
    }
    let mut order = vec![root];
    let mut seen: HashSet<Eid> = HashSet::from([root]);
    let mut i = 0;
    while i < order.len() {
        let e = order[i].oid().raw();
        i += 1;
        let mut found: BTreeSet<i64> = BTreeSet::new();
        for pat in [
            Pattern::new(Some(e), None, None),
            Pattern::new(None, None, Some(e)),
        ] {
            for r in view.scan(store, pat)? {
                found.insert(r?.eid);
            }
        }
        for raw in found {
            if let Some(x) = Eid::from_oid(ObjectId::from_raw(raw)) {
                if seen.insert(x) {
                    order.push(x);
                }
            }
        }
    }
    Ok(order)
}

fn graph_terms(store: &dyn ReadStore) -> Result<[Option<ObjectId>; 3]> {
    use tm_core::vocab;
    let mut out = [None; 3];
    for (slot, iri) in out
        .iter_mut()
        .zip([vocab::SYS_IN_GRAPH, vocab::RDF_TYPE, vocab::SYS_GRAPH])
    {
        *slot = dict::encode(store, &Value::iri(iri))?;
    }
    Ok(out)
}

/// The eids of the statements that are members of `graph` in `spec`: the member
/// statement and its `sys:inGraph` membership are both selected by the view.
pub fn graph_members(store: &dyn ReadStore, spec: &ViewSpec, graph: ObjectId) -> Result<Vec<Eid>> {
    let [Some(ig), _, _] = graph_terms(store)? else {
        return Ok(Vec::new());
    };
    let view = View::resolve(store, spec)?;
    let mut out: BTreeSet<i64> = BTreeSet::new();
    for m in view.scan(store, Pattern::new(None, Some(ig.raw()), Some(graph.raw())))? {
        let m = m?;
        if let Some(a) = store.statement(m.s)? {
            if view.selects(&a) {
                out.insert(a.eid);
            }
        }
    }
    Ok(out
        .into_iter()
        .filter_map(|e| Eid::from_oid(ObjectId::from_raw(e)))
        .collect())
}

/// The graphs of `spec`: every graph with a selected membership of a selected
/// statement, plus every declared `(g rdf:type sys:Graph)`, ascending.
pub fn graphs(store: &dyn ReadStore, spec: &ViewSpec) -> Result<Vec<ObjectId>> {
    let [ig, ty, sg] = graph_terms(store)?;
    let view = View::resolve(store, spec)?;
    let mut out: BTreeSet<i64> = BTreeSet::new();
    if let Some(ig) = ig {
        for m in view.scan(store, Pattern::new(None, Some(ig.raw()), None))? {
            let m = m?;
            if let Some(a) = store.statement(m.s)? {
                if view.selects(&a) {
                    out.insert(m.o);
                }
            }
        }
    }
    if let (Some(ty), Some(sg)) = (ty, sg) {
        for d in view.scan(store, Pattern::new(None, Some(ty.raw()), Some(sg.raw())))? {
            out.insert(d?.s);
        }
    }
    Ok(out.into_iter().map(ObjectId::from_raw).collect())
}

struct StoreSource<'a> {
    store: &'a dyn ReadStore,
    view: View,
    terms: TermCache,
}

impl BundleSource for StoreSource<'_> {
    fn visible(&mut self, eid: Eid) -> Result<bool> {
        visible(self.store, &self.view, eid)
    }

    fn row(&mut self, eid: Eid) -> Result<Option<(ObjectId, ObjectId, ObjectId, Valid)>> {
        Ok(self
            .store
            .statement(eid.oid().raw())?
            .filter(|r| self.view.selects(r))
            .map(|r| {
                (
                    ObjectId::from_raw(r.s),
                    ObjectId::from_raw(r.p),
                    ObjectId::from_raw(r.o),
                    Valid {
                        from: r.v_from,
                        to: r.v_to,
                    },
                )
            }))
    }

    fn dependents(&mut self, root: Eid) -> Result<Vec<Eid>> {
        dependents_in(self.store, &self.view, root)
    }

    fn decode(&mut self, id: ObjectId) -> Result<Value> {
        self.terms.decode(self.store, id, true)
    }
}

/// The fact bundle of `root` in `spec` (see `tm_core::read::bundle`).
pub fn bundle(store: &dyn ReadStore, spec: &ViewSpec, root: Eid) -> Result<tm_core::Bundle> {
    let view = View::resolve(store, spec)?;
    tm_core::bundle::export_from(
        &mut StoreSource {
            store,
            view,
            terms: TermCache::new(256),
        },
        root,
    )
}

/// The current `@vocab` and prefix table (`tm_core::mapping::read_settings` on a
/// snapshot).
pub fn settings(store: &dyn ReadStore) -> Result<tm_core::mapping::Settings> {
    use tm_core::vocab;
    let now = ViewSpec::NOW;
    let terms = TermCache::new(64);
    let id = |iri: &str| dict::encode(store, &Value::iri(iri));
    let Some(db) = id(vocab::SYS_DB)? else {
        return Ok((None, Vec::new()));
    };
    let mut vocab_iri = None;
    if let Some(p) = id(vocab::SYS_VOCAB)? {
        for t in triples(store, &now, Some(db), Some(p), None)? {
            if let Value::Iri(s) = terms.decode(store, t.o, false)? {
                vocab_iri = Some(s);
            }
        }
    }
    let mut prefixes = Vec::new();
    let sys = |n: &str| format!("{}{n}", vocab::SYS);
    if let (Some(pp), Some(pn), Some(pi)) = (
        id(&sys("prefix"))?,
        id(&sys("prefixName"))?,
        id(&sys("prefixIri"))?,
    ) {
        for t in triples(store, &now, Some(db), Some(pp), None)? {
            let names = triples(store, &now, Some(t.o), Some(pn), None)?;
            let iris = triples(store, &now, Some(t.o), Some(pi), None)?;
            if let (Some(n), Some(i)) = (names.first(), iris.first()) {
                let n = terms.decode(store, n.o, false)?;
                let i = terms.decode(store, i.o, false)?;
                if let (Value::Str(n), Value::Iri(i)) = (n, i) {
                    prefixes.push((n, i));
                }
            }
        }
    }
    Ok((vocab_iri, prefixes))
}
