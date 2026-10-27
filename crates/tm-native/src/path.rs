//! Paths on the native runner: the path engine of `tm-exec` (automaton, search
//! modes, time respect, completeness) reading the graph through the store's
//! permutations instead of SQL, and the path node evaluated per input row as
//! `tm_path` is.

use std::collections::{HashMap, HashSet};

use tm_core::store::ReadStore;
use tm_core::vocab::{RDF, SYS};
use tm_core::{Error, ObjectId, Result, Tag, Value, ViewSpec, Vocab};
use tm_exec::path::automaton::Dir;
use tm_exec::path::engine::{PathRequest, PathSource};
use tm_exec::path::fetch::{Nb, NeighbourFetch};
use tm_exec::path::resolve::{Fetch, RLetter};
use tm_exec::path::row::{virtual_pred_id, HopKind};
use tm_exec::plan::analyze::{Dom, VClass};
use tm_exec::plan::{PGraphs, PPath, PTerm};
use tm_exec::result::RouteNote;
use tm_exec::scan::ResolvedView;
use tm_ir::PathMode;
use tm_store::{Pattern, StmtRow, View};

use crate::exec::{store_view, Exec};
use crate::rel::{Col, Rel};
use crate::value::V;

/// [`PathSource`] on a snapshot.
pub struct StorePathSource<'a>(pub &'a dyn ReadStore);

impl PathSource for StorePathSource<'_> {
    fn vocab(&mut self) -> Result<Vocab> {
        let (v, prefixes) = tm_store::read::settings(self.0)?;
        let mut out = Vocab::default();
        if let Some(v) = v {
            out.vocab = v;
        }
        out.prefixes = prefixes;
        Ok(out)
    }

    fn encode_iri(&mut self, iri: &str) -> Result<Option<ObjectId>> {
        tm_store::dict::encode(self.0, &Value::iri(iri))
    }

    fn resolve_view(&mut self, spec: ViewSpec) -> Result<Option<ResolvedView>> {
        let mut src = crate::source::StoreSource(self.0);
        tm_exec::plan::resolve::resolve_with(&mut src, &spec.into())
    }

    fn fetcher<'s>(
        &'s mut self,
        view: ResolvedView,
        _batch: usize,
        graphs: Option<&[ObjectId]>,
    ) -> Result<Box<dyn NeighbourFetch + 's>> {
        let scope = match graphs {
            None => Scope::All,
            Some([]) => Scope::Nothing,
            Some(gs) => {
                match tm_store::dict::encode(self.0, &Value::iri(tm_core::vocab::SYS_IN_GRAPH))? {
                    None => Scope::Nothing,
                    Some(ig) => Scope::Graphs {
                        in_graph: ig.raw(),
                        ids: gs.iter().map(|g| g.raw()).collect(),
                    },
                }
            }
        };
        Ok(Box::new(StoreFetch {
            store: self.0,
            view: store_view(&view),
            scope,
            rv: None,
            sys: HashMap::new(),
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    All,
    Graphs { in_graph: i64, ids: HashSet<i64> },
    Nothing,
}

struct Rv {
    edge_true: HashSet<i64>,
    edge_false: HashSet<i64>,
    rdf_type: Option<i64>,
}

/// Neighbours read from the permutations (the native [`NeighbourFetch`]).
struct StoreFetch<'a> {
    store: &'a dyn ReadStore,
    view: View,
    scope: Scope,
    rv: Option<Rv>,
    sys: HashMap<i64, bool>,
}

impl StoreFetch<'_> {
    fn load_rv(&mut self) -> Result<()> {
        if self.rv.is_some() {
            return Ok(());
        }
        let enc = |iri: &str| tm_store::dict::encode(self.store, &Value::iri(iri));
        let mut rv = Rv {
            edge_true: HashSet::new(),
            edge_false: HashSet::new(),
            rdf_type: enc(&format!("{RDF}type"))?.map(ObjectId::raw),
        };
        if let Some(p) = enc(tm_core::vocab::SYS_IS_EDGE)? {
            let (yes, no) = (
                ObjectId::from_unsigned(Tag::Bool, 1).raw(),
                ObjectId::from_unsigned(Tag::Bool, 0).raw(),
            );
            for r in self
                .view
                .scan_eid(self.store, Pattern::new(None, Some(p.raw()), None))?
            {
                rv.edge_true.remove(&r.s);
                rv.edge_false.remove(&r.s);
                if r.o == yes {
                    rv.edge_true.insert(r.s);
                } else if r.o == no {
                    rv.edge_false.insert(r.s);
                }
            }
        }
        self.rv = Some(rv);
        Ok(())
    }

    fn is_sys(&mut self, p: i64) -> Result<bool> {
        if let Some(b) = self.sys.get(&p) {
            return Ok(*b);
        }
        let id = ObjectId::from_raw(p);
        let b = id.tag_bits() == Tag::Iri as u8
            && self
                .store
                .term(id.unsigned_payload() as i64)?
                .is_some_and(|t| t.lex.starts_with(SYS));
        self.sys.insert(p, b);
        Ok(b)
    }

    fn passes(&mut self, p: i64, o: i64) -> Result<bool> {
        let sys = self.is_sys(p)?;
        let rv = self.rv.as_ref().expect("loaded");
        if Some(p) == rv.rdf_type || sys || rv.edge_false.contains(&p) {
            return Ok(false);
        }
        Ok(rv.edge_true.contains(&p) || ObjectId::from_raw(o).tag().is_ok_and(|t| t.is_subject()))
    }

    fn in_scope(&self, eid: i64) -> Result<bool> {
        Ok(match &self.scope {
            Scope::All => true,
            Scope::Nothing => false,
            Scope::Graphs { in_graph, ids } => {
                let mut found = false;
                for m in self
                    .view
                    .scan(self.store, Pattern::new(Some(eid), Some(*in_graph), None))?
                {
                    if ids.contains(&m?.o) {
                        found = true;
                        break;
                    }
                }
                found
            }
        })
    }

    /// `(from, eid, to, p, row)` of the statements one letter steps over from `n`.
    fn rows(&self, letter: &RLetter, n: i64) -> Result<Vec<(i64, i64, i64, StmtRow)>> {
        let scan = |pat: Pattern| -> Result<Vec<StmtRow>> { self.view.scan_eid(self.store, pat) };
        Ok(match (&letter.fetch, letter.dir) {
            (Fetch::Dead, _) => Vec::new(),
            (Fetch::Pred { p, .. }, Dir::Out) => scan(Pattern::new(Some(n), Some(p.raw()), None))?
                .into_iter()
                .map(|r| (r.s, r.eid, r.o, r))
                .collect(),
            (Fetch::Pred { p, .. }, Dir::In) => scan(Pattern::new(None, Some(p.raw()), Some(n)))?
                .into_iter()
                .map(|r| (r.o, r.eid, r.s, r))
                .collect(),
            (Fetch::Other { .. }, Dir::Out) => scan(Pattern::new(Some(n), None, None))?
                .into_iter()
                .map(|r| (r.s, r.eid, r.o, r))
                .collect(),
            (Fetch::Other { .. }, Dir::In) => scan(Pattern::new(None, None, Some(n)))?
                .into_iter()
                .map(|r| (r.o, r.eid, r.s, r))
                .collect(),
            (Fetch::Virtual(k), Dir::Out) => match self.store.statement(n)? {
                Some(r) if self.view.selects(&r) => {
                    let to = match k {
                        HopKind::Subject => r.s,
                        HopKind::Object => r.o,
                        _ => r.p,
                    };
                    vec![(r.eid, r.eid, to, r)]
                }
                _ => Vec::new(),
            },
            (Fetch::Virtual(k), Dir::In) => {
                let pat = match k {
                    HopKind::Subject => Pattern::new(Some(n), None, None),
                    HopKind::Object => Pattern::new(None, None, Some(n)),
                    _ => Pattern::new(None, Some(n), None),
                };
                scan(pat)?
                    .into_iter()
                    .map(|r| (n, r.eid, r.eid, r))
                    .collect()
            }
        })
    }
}

impl NeighbourFetch for StoreFetch<'_> {
    fn fetch(&mut self, letter: &RLetter, nodes: &[i64]) -> Result<HashMap<i64, Vec<Nb>>> {
        let mut out: HashMap<i64, Vec<Nb>> = HashMap::new();
        if matches!(letter.fetch, Fetch::Dead) || self.scope == Scope::Nothing {
            return Ok(out);
        }
        let needs_rv = matches!(
            &letter.fetch,
            Fetch::Other { .. } | Fetch::Pred { rv: Some(_), .. }
        );
        if needs_rv {
            self.load_rv()?;
        }
        let virt = match &letter.fetch {
            Fetch::Virtual(k) => Some(*k),
            _ => None,
        };
        let mut seen: HashSet<i64> = HashSet::new();
        for &n in nodes {
            if !seen.insert(n) {
                continue;
            }
            let keep = match ObjectId::from_raw(n).tag() {
                Err(_) => false,
                Ok(t) => match (&letter.fetch, letter.dir) {
                    (Fetch::Virtual(_), Dir::Out) => t == Tag::Stmt,
                    (_, Dir::Out) => t.is_subject(),
                    _ => true,
                },
            };
            if !keep {
                continue;
            }
            for (from, eid, to, r) in self.rows(letter, n)? {
                if !self.in_scope(r.eid)? {
                    continue;
                }
                let object = if letter.dir == Dir::Out { to } else { from };
                match &letter.fetch {
                    Fetch::Other { excl } => {
                        if excl.binary_search(&r.p).is_ok() || !self.passes(r.p, object)? {
                            continue;
                        }
                    }
                    Fetch::Pred { rv: Some(want), .. } => {
                        if self.passes(r.p, object)? != *want {
                            continue;
                        }
                    }
                    _ => {}
                }
                let (kind, p) = match virt {
                    Some(k) => (k, virtual_pred_id(k).raw()),
                    None => (HopKind::Stored, r.p),
                };
                out.entry(from).or_default().push(Nb {
                    from,
                    eid,
                    to,
                    p,
                    kind,
                    dir: letter.dir,
                    v_from: r.v_from,
                    v_to: r.v_to,
                });
            }
        }
        for v in out.values_mut() {
            v.sort_by_key(Nb::key);
        }
        Ok(out)
    }
}

/// The graphs of a view that hold at least one selected statement (the graph
/// enumeration of a `GRAPH ?g` path no joined pattern binds).
fn view_graphs(ex: &Exec<'_>, p: &PPath) -> Result<Vec<i64>> {
    let Some(ig) = p.in_graph else {
        return Ok(Vec::new());
    };
    let view = store_view(&p.view);
    let mut out: Vec<i64> = Vec::new();
    for m in view.scan(ex.store, Pattern::new(None, Some(ig.raw()), None))? {
        let m = m?;
        if out.contains(&m.o) {
            continue;
        }
        if ex.store.statement(m.s)?.is_some_and(|e| view.selects(&e)) {
            out.push(m.o);
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// Evaluates a path once per row of `acc`, extending each row with the path's
/// outputs (a dependent join, as the SQL route's correlated `tm_path` call).
pub fn eval(ex: &Exec<'_>, p: &PPath, acc: &Rel) -> Result<Rel> {
    let engine = tm_exec::PathEngine::new(tm_exec::PathOptions {
        max_hops: ex.path_max_hops,
        max_states: ex.path_max_states,
        ..tm_exec::PathOptions::default()
    });
    let start_col = match &p.arg {
        PTerm::Id(_) => None,
        PTerm::Var(v) => Some(acc.idx(v).ok_or_else(|| {
            tm_exec::error::unsupported(
                "a path whose bound endpoint is not bound by a pattern joined with it".to_string(),
            )
        })?),
    };
    let mode: PathMode = p
        .mode
        .parse()
        .map_err(|m: String| Error::invalid_query(format!("tm_path: mode: {m}")))?;
    let (max_hops, default_cap) = match p.max_hops {
        None if mode == PathMode::Trail => (Some(engine.options().max_hops), true),
        None => (None, false),
        Some(n) => (Some(n), false),
    };
    let va = tm_exec::path::view::parse_call_view(&p.view_text)
        .map_err(|m| Error::invalid_query(format!("tm_path: view: {m}")))?;
    let mut out = Rel {
        vars: acc.vars.clone(),
        cols: acc.cols.clone(),
        rows: Vec::new(),
        iso: acc.iso.clone(),
    };
    // the graph column: a joined pattern's, or one call per graph of the view
    let (graph_col, enumerate) = match &p.graphs {
        PGraphs::Var(g) => match acc.idx(g) {
            Some(i) => (Some(i), None),
            None => {
                let i = out.push_col(g, Col::term(false), V::Null);
                out.rows.clear();
                (Some(i), Some(view_graphs(ex, p)?))
            }
        },
        _ => (None, None),
    };
    let other_col = match &p.other {
        PTerm::Var(v) => match out.idx(v) {
            Some(i) => Some((i, true)),
            None => Some((out.push_col(v, Col::term(false), V::Null), false)),
        },
        PTerm::Id(_) => None,
    };
    let arrival_col = p.bind_arrival.as_ref().map(|v| {
        out.push_col(
            v,
            Col {
                dom: Dom::Computed(VClass::Int),
                mm: true,
                null: false,
            },
            V::Null,
        )
    });
    let path_col = p.bind_path.as_ref().map(|v| {
        out.push_col(
            v,
            Col {
                dom: Dom::PathJson {
                    reversed: p.note == RouteNote::PathInverted,
                },
                mm: false,
                null: false,
            },
            V::Null,
        )
    });
    out.rows.clear();
    let end = match &p.other {
        PTerm::Id(id) => Some(*id),
        PTerm::Var(_) => None,
    };
    let with_path = mode != PathMode::Reachability;
    let width = out.vars.len();
    for row in &acc.rows {
        let start = match (&p.arg, start_col) {
            (PTerm::Id(id), _) => Some(*id),
            (_, Some(i)) => row[i].as_i64().map(ObjectId::from_raw),
            _ => None,
        };
        let Some(start) = start else { continue };
        let graph_sets: Vec<(Option<i64>, Option<Vec<ObjectId>>)> =
            match (&p.graphs, graph_col, &enumerate) {
                (PGraphs::Any, _, _) => vec![(None, None)],
                (PGraphs::Ids(ids), _, _) => vec![(None, Some(ids.clone()))],
                (PGraphs::Var(_), Some(_), Some(gs)) => gs
                    .iter()
                    .map(|g| (Some(*g), Some(vec![ObjectId::from_raw(*g)])))
                    .collect(),
                (PGraphs::Var(_), Some(i), None) => match row[i].as_i64() {
                    Some(g) => vec![(None, Some(vec![ObjectId::from_raw(g)]))],
                    None => continue,
                },
                (PGraphs::Var(_), None, _) => vec![(None, None)],
            };
        for (gval, graphs) in graph_sets {
            let req = PathRequest {
                start,
                path: &p.text,
                mode,
                max_hops,
                hop_cap: default_cap || va.hop_cap,
                view: va.spec,
                end,
                graphs,
                time_respecting: va.time_respecting,
            };
            let mut src = StorePathSource(ex.store);
            let mut found = Vec::new();
            engine.run_on(&mut src, &req, &mut |r| {
                found.push(r);
                Ok(true)
            })?;
            for r in found {
                let mut o = row.clone();
                o.resize(width, V::Null);
                if let (Some(g), Some(i)) = (gval, graph_col) {
                    o[i] = V::Integer(g);
                }
                if let Some((i, bound)) = other_col {
                    if bound {
                        if o[i].as_i64() != Some(r.end.raw()) {
                            continue;
                        }
                    } else {
                        o[i] = V::Integer(r.end.raw());
                    }
                }
                if let Some(i) = arrival_col {
                    o[i] = r.arrival.map_or(V::Null, V::Integer);
                }
                if let Some(i) = path_col {
                    o[i] = match (&r.path, with_path) {
                        (Some(path), true) => V::Text(path.to_json()),
                        _ => V::Null,
                    };
                }
                out.rows.push(o);
            }
        }
    }
    ex.note(format!(
        "path {} ({:?}, {} rows)",
        p.text,
        mode,
        out.rows.len()
    ));
    Ok(out)
}
