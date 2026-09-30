//! The neighbour fetcher: one batched, prepared statement per (hop shape, view
//! shape), fed through `rarray(?1)` in chunks. The SQL text holds no data (ids, the
//! predicate, the graph set and the view's `t`/`d` are parameters), and every time
//! predicate comes from the view-predicate function of `crate::scan`.

use std::collections::{HashMap, HashSet};

use tm_core::vocab::{RDF, SYS};
use tm_core::{Executor, ObjectId, Result, SqlValue, Tag, TermReader, Value};

use super::automaton::Dir;
use super::resolve::{Fetch, RLetter};
use super::row::{virtual_pred_id, HopKind};
use crate::scan::{view_predicates, ResolvedView};
use crate::sqlgen::ParamAlloc;

/// Default number of frontier nodes per `rarray` chunk.
pub const DEFAULT_BATCH: usize = 256;

/// One neighbour: a hop from `from` to `to`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Nb {
    /// The frontier node (raw id).
    pub from: i64,
    /// The statement traversed (raw id).
    pub eid: i64,
    /// The node reached (raw id).
    pub to: i64,
    /// The predicate (the reserved virtual id for a virtual hop).
    pub p: i64,
    /// Stored or virtual.
    pub kind: HopKind,
    /// The direction of the hop.
    pub dir: Dir,
}

impl Nb {
    /// The ordering key of the hop: eid, then kind, then direction.
    pub fn key(&self) -> (i64, u8, Dir) {
        (self.eid, self.kind as u8, self.dir)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum Shape {
    Pred(Dir),
    Any(Dir),
    Virt(HopKind, Dir),
}

struct Built {
    sql: String,
    params: Vec<SqlValue>,
}

/// The relationship-view classification (mirrors the Cypher dual view).
struct Rv {
    edge_true: HashSet<i64>,
    edge_false: HashSet<i64>,
    rdf_type: Option<i64>,
    sys: HashSet<i64>,
}

impl Rv {
    fn passes(&self, p: i64, o: i64) -> bool {
        if Some(p) == self.rdf_type || self.sys.contains(&p) || self.edge_false.contains(&p) {
            return false;
        }
        self.edge_true.contains(&p) || ObjectId::from_raw(o).tag().is_ok_and(|t| t.is_subject())
    }
}

/// The graph set of a graph-scoped search.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    /// No graph filter.
    All,
    /// Every traversed statement needs a membership `(t.eid sys:inGraph g)`, `g` in
    /// `ids`, visible in the view.
    Graphs { in_graph: i64, ids: Vec<i64> },
    /// No statement can be in the set (empty set, or `sys:inGraph` never written):
    /// no hop is taken.
    Nothing,
}

/// Fetches the neighbours of frontier nodes under one view.
// @lat: [[query#Physical Planning#Path Engine]]
pub struct Fetcher<'a> {
    exec: &'a mut dyn Executor,
    view: ResolvedView,
    batch: usize,
    built: HashMap<Shape, Built>,
    rv: Option<Rv>,
    scope: Scope,
}

impl<'a> Fetcher<'a> {
    /// A fetcher reading `view` on `exec`.
    pub fn new(exec: &'a mut dyn Executor, view: ResolvedView) -> Fetcher<'a> {
        Fetcher {
            exec,
            view,
            batch: DEFAULT_BATCH,
            built: HashMap::new(),
            rv: None,
            scope: Scope::All,
        }
    }

    /// Restricts every hop to statements that are members of at least one of
    /// `graphs` in the fetcher's view (`None`: no restriction). Graph-scoped
    /// evaluation, `lat.md/query#Physical Planning#Path Engine`.
    ///
    /// # Errors
    ///
    /// A database error while looking up `sys:inGraph`.
    pub fn with_graphs(mut self, graphs: Option<&[ObjectId]>) -> Result<Fetcher<'a>> {
        self.built.clear();
        self.scope = match graphs {
            None => Scope::All,
            Some([]) => Scope::Nothing,
            Some(gs) => {
                match TermReader::encode(self.exec, &Value::iri(tm_core::vocab::SYS_IN_GRAPH))? {
                    None => Scope::Nothing,
                    Some(ig) => {
                        let mut ids: Vec<i64> = gs.iter().map(|g| g.raw()).collect();
                        ids.sort_unstable();
                        ids.dedup();
                        Scope::Graphs {
                            in_graph: ig.raw(),
                            ids,
                        }
                    }
                }
            }
        };
        Ok(self)
    }

    /// Sets the chunk size (tests and tuning).
    pub fn with_batch(mut self, n: usize) -> Fetcher<'a> {
        self.batch = n.max(1);
        self
    }

    /// The SQL text of a hop shape (tests).
    #[doc(hidden)]
    pub fn sql_of(&mut self, letter: &RLetter) -> Option<String> {
        shape_of(&letter.fetch, letter.dir).map(|s| self.build(s).sql.clone())
    }

    fn build(&mut self, shape: Shape) -> &Built {
        if !self.built.contains_key(&shape) {
            let mut pa = ParamAlloc::default();
            let arr = pa.push(SqlValue::IntArray(Vec::new()));
            let mut conds: Vec<String> = Vec::new();
            let cols = match shape {
                Shape::Pred(d) => {
                    let pp = pa.push(SqlValue::Integer(0));
                    conds.push(format!("t.p = {pp}"));
                    match d {
                        Dir::Out => {
                            conds.push("t.s = r.value".to_string());
                            "t.s, t.eid, t.o, t.p"
                        }
                        Dir::In => {
                            conds.push("t.o = r.value".to_string());
                            "t.o, t.eid, t.s, t.p"
                        }
                    }
                }
                Shape::Any(Dir::Out) => {
                    conds.push("t.s = r.value".to_string());
                    "t.s, t.eid, t.o, t.p"
                }
                Shape::Any(Dir::In) => {
                    conds.push("t.o = r.value".to_string());
                    "t.o, t.eid, t.s, t.p"
                }
                Shape::Virt(k, Dir::Out) => {
                    conds.push("t.eid = r.value".to_string());
                    match k {
                        HopKind::Subject => "t.eid, t.eid, t.s, t.p",
                        HopKind::Object => "t.eid, t.eid, t.o, t.p",
                        _ => "t.eid, t.eid, t.p, t.p",
                    }
                }
                Shape::Virt(k, Dir::In) => match k {
                    HopKind::Subject => {
                        conds.push("t.s = r.value".to_string());
                        "t.s, t.eid, t.eid, t.p"
                    }
                    HopKind::Object => {
                        conds.push("t.o = r.value".to_string());
                        "t.o, t.eid, t.eid, t.p"
                    }
                    _ => {
                        conds.push("t.p = r.value".to_string());
                        "t.p, t.eid, t.eid, t.p"
                    }
                },
            };
            conds.extend(view_predicates("t", &self.view, &mut pa));
            if let Scope::Graphs { in_graph, ids } = &self.scope {
                // the statement row `t` is the one traversed, also for a virtual hop
                // (the statement whose part is stepped to or from); the membership is
                // read in the same view, one seek on `(s, p)` of the spo index
                let ig = pa.push(SqlValue::Integer(*in_graph));
                let gs = pa.push(SqlValue::IntArray(ids.clone()));
                let mut member = vec![
                    "m.s = t.eid".to_string(),
                    format!("m.p = {ig}"),
                    format!("m.o IN (SELECT value FROM rarray({gs}))"),
                ];
                member.extend(view_predicates("m", &self.view, &mut pa));
                conds.push(format!(
                    "EXISTS (SELECT 1 FROM triple AS m WHERE {})",
                    member.join(" AND ")
                ));
            }
            // the array drives the join: SQLite then probes the index of `t` once per
            // node instead of scanning `t` against an `IN` list
            let sql = format!(
                "SELECT {cols} FROM rarray({arr}) AS r CROSS JOIN triple AS t WHERE {}",
                conds.join(" AND ")
            );
            self.built.insert(
                shape,
                Built {
                    sql,
                    params: pa.values().to_vec(),
                },
            );
        }
        &self.built[&shape]
    }

    fn load_rv(&mut self) -> Result<()> {
        if self.rv.is_some() {
            return Ok(());
        }
        let mut rv = Rv {
            edge_true: HashSet::new(),
            edge_false: HashSet::new(),
            rdf_type: TermReader::encode(self.exec, &Value::iri(format!("{RDF}type")))?
                .map(ObjectId::raw),
            sys: HashSet::new(),
        };
        if let Some(p) = TermReader::encode(self.exec, &Value::iri(tm_core::vocab::SYS_IS_EDGE))? {
            let mut pa = ParamAlloc::default();
            let pp = pa.push(SqlValue::Integer(p.raw()));
            let mut conds = vec![format!("t.p = {pp}")];
            conds.extend(view_predicates("t", &self.view, &mut pa));
            let sql = format!(
                "SELECT t.s, t.o FROM triple AS t WHERE {} ORDER BY t.eid",
                conds.join(" AND ")
            );
            let (yes, no) = (
                ObjectId::from_unsigned(Tag::Bool, 1).raw(),
                ObjectId::from_unsigned(Tag::Bool, 0).raw(),
            );
            self.exec.query(&sql, pa.values(), &mut |r| {
                if let (Some(s), Some(o)) = (r[0].as_i64(), r[1].as_i64()) {
                    rv.edge_true.remove(&s);
                    rv.edge_false.remove(&s);
                    if o == yes {
                        rv.edge_true.insert(s);
                    } else if o == no {
                        rv.edge_false.insert(s);
                    }
                }
                Ok(())
            })?;
        }
        let hi = format!("{}{}", &SYS[..SYS.len() - 1], ";");
        self.exec.query(
            "SELECT id FROM term WHERE tag = 0 AND lex >= ?1 AND lex < ?2",
            &[SqlValue::Text(SYS.to_string()), SqlValue::Text(hi)],
            &mut |r| {
                if let Some(id) = r[0].as_i64() {
                    rv.sys
                        .insert(ObjectId::from_unsigned(Tag::Iri, id as u64).raw());
                }
                Ok(())
            },
        )?;
        self.rv = Some(rv);
        Ok(())
    }

    /// The neighbours of `nodes` (distinct raw ids) over `letter`, grouped by
    /// frontier node, each group sorted by [`Nb::key`].
    pub fn fetch(&mut self, letter: &RLetter, nodes: &[i64]) -> Result<HashMap<i64, Vec<Nb>>> {
        let mut out: HashMap<i64, Vec<Nb>> = HashMap::new();
        let Some(shape) = shape_of(&letter.fetch, letter.dir) else {
            return Ok(out);
        };
        if self.scope == Scope::Nothing {
            return Ok(out);
        }
        // nodes that can never take part in this hop are not sent
        let keep = |raw: i64| match ObjectId::from_raw(raw).tag() {
            Err(_) => false,
            Ok(t) => match (&letter.fetch, letter.dir) {
                (Fetch::Virtual(_), Dir::Out) => t == Tag::Stmt,
                (_, Dir::Out) => t.is_subject(),
                _ => true,
            },
        };
        let nodes: Vec<i64> = nodes.iter().copied().filter(|n| keep(*n)).collect();
        if nodes.is_empty() {
            return Ok(out);
        }
        let needs_rv = matches!(
            &letter.fetch,
            Fetch::Other { .. } | Fetch::Pred { rv: Some(_), .. }
        );
        if needs_rv {
            self.load_rv()?;
        }
        let (sql, template) = {
            let b = self.build(shape);
            (b.sql.clone(), b.params.clone())
        };
        let (dir, batch) = (letter.dir, self.batch);
        let rv = self.rv.as_ref();
        for chunk in nodes.chunks(batch) {
            let mut params = template.clone();
            params[0] = SqlValue::IntArray(chunk.to_vec());
            if let Fetch::Pred { p, .. } = &letter.fetch {
                params[1] = SqlValue::Integer(p.raw());
            }
            let virt = match &letter.fetch {
                Fetch::Virtual(k) => Some(*k),
                _ => None,
            };
            self.exec.query(&sql, &params, &mut |r| {
                let (Some(from), Some(eid), Some(to), Some(p)) =
                    (r[0].as_i64(), r[1].as_i64(), r[2].as_i64(), r[3].as_i64())
                else {
                    return Ok(());
                };
                let object = if dir == Dir::Out { to } else { from };
                match &letter.fetch {
                    Fetch::Other { excl } => {
                        let ok = excl.binary_search(&p).is_err()
                            && rv.is_some_and(|rv| rv.passes(p, object));
                        if !ok {
                            return Ok(());
                        }
                    }
                    Fetch::Pred { rv: Some(want), .. } => {
                        if rv.is_some_and(|rv| rv.passes(p, object)) != *want {
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let (kind, p) = match virt {
                    Some(k) => (k, virtual_pred_id(k).raw()),
                    None => (HopKind::Stored, p),
                };
                out.entry(from).or_default().push(Nb {
                    from,
                    eid,
                    to,
                    p,
                    kind,
                    dir,
                });
                Ok(())
            })?;
        }
        for v in out.values_mut() {
            v.sort_by_key(Nb::key);
        }
        Ok(out)
    }
}

fn shape_of(f: &Fetch, dir: Dir) -> Option<Shape> {
    match f {
        Fetch::Dead => None,
        Fetch::Pred { .. } => Some(Shape::Pred(dir)),
        Fetch::Other { .. } => Some(Shape::Any(dir)),
        Fetch::Virtual(k) => Some(Shape::Virt(*k, dir)),
    }
}
