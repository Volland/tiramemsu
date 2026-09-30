//! Building the plan tree: constant encoding, view resolution, virtual and
//! volatile classification, set-semantics decisions, and Empty propagation
//! (design D4):
//!
//! | Operator | Result over an Empty input |
//! |---|---|
//! | Join | Empty if any input is Empty |
//! | Filter, Extend, Project, OrderLimit, RowNumber, Unnest | Empty |
//! | Union | Empty branches dropped; Empty if none remain |
//! | LeftJoin | right Empty: the left side with right-only variables missing; left Empty: Empty |
//! | Aggregate | grouped: Empty; group-less: one constant row |
//! | `Exists(Empty)` | constant false (true when negated) |

use std::collections::HashMap;

use tm_core::{codec, Executor, ObjectId, Result, SqlValue, Tag, Value};
use tm_ir::display::path_text_canonical;
use tm_ir::validate::scope;
use tm_ir::{
    AggFunc, Expr, GraphSet, IrQuery, Op, PathPattern, Semantics, TermOrVar, TriplePattern, TxSel,
    ValidSel, Var, VarSet, View,
};

use super::encode::{encode_value, position_ok, value_position_ok, Enc, Pos};
use super::resolve::resolve;
use super::route::{bound_before, bound_by_non_paths, orient, path_patterns, Orientation};
use super::{
    Cell, Node, PAgg, PConst, PExpr, PKey, PLookup, PObj, PPath, PTerm, PTriple, PValues, PVirtual,
    PVolatile,
};
use crate::error::invalid;
use crate::path::ast::nullable;
use crate::result::RouteNote;
use crate::scan::{view_text, ResolvedView};
use crate::virtual_pred::VirtualPred;

/// Payloads from here up are plan-local ids of constants missing from the
/// dictionary (term ids never get this large).
pub const SYNTHETIC_BASE: u64 = 1 << 58;

/// Builds the plan tree of one query on the executing connection.
pub struct Planner<'e> {
    exec: &'e mut dyn Executor,
    sem: Semantics,
    views: HashMap<View, Option<ResolvedView>>,
    consts: HashMap<String, Enc>,
    pred_multi: HashMap<i64, bool>,
    elide_all: bool,
    bound: VarSet,
    all_paths: Vec<PathPattern>,
    /// Plan-local ids of constants missing from the dictionary.
    pub synthetic: HashMap<i64, Value>,
}

/// True when the root is `Project{distinct}` over only Join, Filter and triple
/// patterns: the outer DISTINCT removes duplicates, so no canonical-eid predicate
/// is needed (design D10).
fn distinct_over_bgp(root: &Op) -> bool {
    fn only_bgp(op: &Op) -> bool {
        match op {
            Op::Triple(_) => true,
            Op::Join(j) => j.null_safe.is_empty() && j.inputs.iter().all(only_bgp),
            Op::Filter(f) => only_bgp(&f.input),
            _ => false,
        }
    }
    matches!(root, Op::Project(p) if p.distinct && only_bgp(&p.input))
}

/// A classified path endpoint.
enum Ep {
    Term(PTerm),
    /// A constant that is not in the dictionary.
    Missing(Value),
    /// A term that cannot appear in a statement position.
    Never,
}

impl<'e> Planner<'e> {
    /// A planner for `q` reading through `exec`.
    pub fn new(exec: &'e mut dyn Executor, q: &IrQuery) -> Planner<'e> {
        Planner {
            exec,
            sem: q.semantics,
            views: HashMap::new(),
            consts: HashMap::new(),
            pred_multi: HashMap::new(),
            elide_all: distinct_over_bgp(&q.root),
            bound: bound_by_non_paths(&q.root),
            all_paths: path_patterns(&q.root),
            synthetic: HashMap::new(),
        }
    }

    fn view(&mut self, v: &View) -> Result<Option<ResolvedView>> {
        if let Some(r) = self.views.get(v) {
            return Ok(*r);
        }
        let r = resolve(&mut *self.exec, v)?;
        self.views.insert(*v, r);
        Ok(r)
    }

    fn encode(&mut self, v: &Value) -> Result<Enc> {
        let k = format!("{v:?}");
        if let Some(e) = self.consts.get(&k) {
            return Ok(e.clone());
        }
        let e = encode_value(&mut *self.exec, v)?;
        self.consts.insert(k, e.clone());
        Ok(e)
    }

    fn synthetic_id(&mut self, v: &Value) -> ObjectId {
        if let Some((k, _)) = self.synthetic.iter().find(|(_, x)| *x == v) {
            return ObjectId::from_raw(*k);
        }
        let tag = match codec::encode(v) {
            codec::Encoded::Term(t) => t.tag,
            codec::Encoded::Inline(id) => return id,
        };
        let id = ObjectId::from_unsigned(tag, SYNTHETIC_BASE + self.synthetic.len() as u64);
        self.synthetic.insert(id.raw(), v.clone());
        id
    }

    fn is_multi(&mut self, p: ObjectId) -> Result<bool> {
        if let Some(m) = self.pred_multi.get(&p.raw()) {
            return Ok(*m);
        }
        let m = self
            .exec
            .query_i64(
                "SELECT 1 FROM pred_multi WHERE p = ?1",
                &[SqlValue::Integer(p.raw())],
            )?
            .is_some();
        self.pred_multi.insert(p.raw(), m);
        Ok(m)
    }

    /// Encodes a pattern position; `None` when nothing can match.
    fn pterm(&mut self, t: &TermOrVar, pos: Pos) -> Result<Option<PTerm>> {
        Ok(match t {
            TermOrVar::Var(v) => Some(PTerm::Var(v.clone())),
            TermOrVar::Id(id) => position_ok(*id, pos).then_some(PTerm::Id(*id)),
            TermOrVar::Const(v) => {
                if !value_position_ok(v, pos) {
                    return Ok(None);
                }
                match self.encode(v)? {
                    Enc::Id(id) if position_ok(id, pos) => Some(PTerm::Id(id)),
                    _ => None,
                }
            }
            TermOrVar::Param(p) => return Err(invalid(format!("unbound parameter `{p}`"))),
        })
    }

    /// Classifies a path endpoint.
    fn endpoint(&mut self, t: &TermOrVar) -> Result<Ep> {
        Ok(match t {
            TermOrVar::Const(v) if value_position_ok(v, Pos::Object) => match self.encode(v)? {
                Enc::Id(id) if position_ok(id, Pos::Object) => Ep::Term(PTerm::Id(id)),
                Enc::Missing(m) => Ep::Missing(m),
                _ => Ep::Never,
            },
            other => match self.pterm(other, Pos::Object)? {
                Some(t) => Ep::Term(t),
                None => Ep::Never,
            },
        })
    }

    /// A path with an endpoint constant that is in no statement: only a nullable
    /// expression can match, by the zero-length path, so the other endpoint is
    /// bound to that very term (`:nobody :p* ?x` gives `?x = :nobody`). Paths whose
    /// value is asked for (`bind_path`) stay empty.
    fn absent_endpoints(&mut self, p: &PathPattern, s: Ep, e: Ep, empty: Node) -> Result<Node> {
        if !nullable(&p.path) || p.bind_path.is_some() {
            return Ok(empty);
        }
        let bind = |this: &mut Self, var: &Var, term: &Value| -> Node {
            let id = this.synthetic_id(&term.canonical());
            Node::Values(PValues {
                vars: vec![var.clone()],
                rows: vec![vec![Some(Cell::Id(id))]],
            })
        };
        Ok(match (s, e) {
            (Ep::Missing(a), Ep::Missing(b)) if a == b => Node::Join(Vec::new(), Vec::new()),
            (Ep::Missing(c), Ep::Term(PTerm::Var(v)))
            | (Ep::Term(PTerm::Var(v)), Ep::Missing(c)) => bind(self, &v, &c),
            _ => empty,
        })
    }

    /// Plans the whole tree.
    pub fn plan(&mut self, op: &Op) -> Result<Node> {
        let empty = || Node::Empty(scope(op).vars);
        Ok(match op {
            Op::Triple(t) => self.triple(t)?.unwrap_or_else(empty),
            Op::Path(p) => {
                let Some(view) = self.view(&p.view)? else {
                    return Ok(empty());
                };
                let (start, end) = match (self.endpoint(&p.start)?, self.endpoint(&p.end)?) {
                    (Ep::Term(s), Ep::Term(e)) => (s, e),
                    (s, e) => return self.absent_endpoints(p, s, e, empty()),
                };
                let (arg, other, text, note) =
                    match orient(p, &bound_before(&self.bound, &self.all_paths, p))? {
                        Orientation::Forward => (
                            start,
                            end,
                            path_text_canonical(&p.path),
                            RouteNote::PathForward,
                        ),
                        Orientation::Inverted => (
                            end,
                            start,
                            path_text_canonical(&p.path.clone().inverse()),
                            RouteNote::PathInverted,
                        ),
                    };
                Node::Path(PPath {
                    arg,
                    other,
                    text,
                    mode: p.mode.sql_name(),
                    max_hops: p.max_hops,
                    bind_path: p.bind_path.clone(),
                    view_text: view_text(&view),
                    note,
                })
            }
            Op::Values(v) => {
                if v.rows.is_empty() {
                    return Ok(empty());
                }
                let mut rows = Vec::with_capacity(v.rows.len());
                for r in &v.rows {
                    let mut cells = Vec::with_capacity(r.len());
                    for c in r {
                        cells.push(match c {
                            None => None,
                            Some(TermOrVar::Id(id)) => Some(Cell::Id(*id)),
                            Some(TermOrVar::Const(val)) => {
                                Some(Cell::Id(match self.encode(val)? {
                                    Enc::Id(id) => id,
                                    _ => self.synthetic_id(&val.canonical()),
                                }))
                            }
                            Some(other) => {
                                return Err(invalid(format!("invalid Values cell {other:?}")))
                            }
                        });
                    }
                    rows.push(cells);
                }
                Node::Values(PValues {
                    vars: v.vars.clone(),
                    rows,
                })
            }
            Op::Unnest(u) => {
                let input = self.plan(&u.input)?;
                if matches!(input, Node::Empty(_)) {
                    return Ok(empty());
                }
                if let Expr::List(xs) = &u.list {
                    if xs.iter().all(|x| matches!(x, Expr::Const(_))) {
                        if xs.is_empty() {
                            return Ok(empty());
                        }
                        let mut rows = Vec::new();
                        for x in xs {
                            let Expr::Const(v) = x else { unreachable!() };
                            let id = match self.encode(v)? {
                                Enc::Id(id) => id,
                                _ => self.synthetic_id(&v.canonical()),
                            };
                            rows.push(vec![Some(Cell::Id(id))]);
                        }
                        let values = Node::Values(PValues {
                            vars: vec![u.var.clone()],
                            rows,
                        });
                        return Ok(Node::Join(vec![input, values], Vec::new()));
                    }
                }
                let list = self.expr(&u.list)?;
                Node::Unnest(Box::new(input), list, u.var.clone())
            }
            Op::Join(j) => {
                let mut inputs = Vec::new();
                for i in &j.inputs {
                    match self.plan(i)? {
                        Node::Empty(_) => return Ok(empty()),
                        Node::Join(inner, ns) if ns.is_empty() => inputs.extend(inner),
                        n => inputs.push(n),
                    }
                }
                if inputs.len() == 1 && j.null_safe.is_empty() {
                    inputs.pop().expect("one")
                } else {
                    Node::Join(inputs, j.null_safe.clone())
                }
            }
            Op::LeftJoin(l) => {
                let left = self.plan(&l.left)?;
                if matches!(left, Node::Empty(_)) {
                    return Ok(empty());
                }
                let right = self.plan(&l.right)?;
                if matches!(right, Node::Empty(_)) {
                    let lv = scope(&l.left);
                    let pad: Vec<Var> = scope(&l.right)
                        .vars
                        .into_iter()
                        .filter(|v| !lv.binds(v))
                        .collect();
                    return Ok(pad_missing(left, pad));
                }
                let cond = l.cond.as_ref().map(|c| self.expr(c)).transpose()?;
                Node::LeftJoin(Box::new(left), Box::new(right), cond)
            }
            Op::Filter(f) => {
                let input = self.plan(&f.input)?;
                if matches!(input, Node::Empty(_)) {
                    return Ok(empty());
                }
                Node::Filter(Box::new(input), self.expr(&f.cond)?)
            }
            Op::Union(u) => {
                let all = scope(op).vars;
                let mut kept = Vec::new();
                let mut kept_vars = VarSet::new();
                for i in &u.inputs {
                    let n = self.plan(i)?;
                    if !matches!(n, Node::Empty(_)) {
                        kept_vars.extend(scope(i).vars);
                        kept.push(n);
                    }
                }
                if kept.is_empty() {
                    return Ok(Node::Empty(all));
                }
                let pad: Vec<Var> = all.into_iter().filter(|v| !kept_vars.contains(v)).collect();
                let n = if kept.len() == 1 {
                    kept.pop().expect("one")
                } else {
                    Node::Union(kept)
                };
                pad_missing(n, pad)
            }
            Op::Extend(e) => {
                let input = self.plan(&e.input)?;
                if matches!(input, Node::Empty(_)) {
                    return Ok(empty());
                }
                Node::Extend(Box::new(input), e.var.clone(), self.expr(&e.expr)?)
            }
            Op::Aggregate(a) => {
                let input = self.plan(&a.input)?;
                if matches!(input, Node::Empty(_)) {
                    if !a.group.is_empty() {
                        return Ok(empty());
                    }
                    let row = a
                        .aggs
                        .iter()
                        .map(|g| match g.func {
                            AggFunc::Count => Some(Cell::Int(0)),
                            AggFunc::Collect => Some(Cell::EmptyList),
                            _ => None,
                        })
                        .collect();
                    return Ok(Node::Values(PValues {
                        vars: a.aggs.iter().map(|g| g.var.clone()).collect(),
                        rows: vec![row],
                    }));
                }
                let mut aggs = Vec::new();
                for g in &a.aggs {
                    aggs.push(PAgg {
                        var: g.var.clone(),
                        func: g.func.clone(),
                        arg: g.arg.as_ref().map(|x| self.expr(x)).transpose()?,
                        distinct: g.distinct,
                    });
                }
                Node::Aggregate(Box::new(input), a.group.clone(), aggs)
            }
            Op::Project(p) => {
                let input = self.plan(&p.input)?;
                if matches!(input, Node::Empty(_)) {
                    return Ok(empty());
                }
                Node::Project(Box::new(input), p.vars.clone(), p.distinct)
            }
            Op::OrderLimit(o) => {
                let input = self.plan(&o.input)?;
                if matches!(input, Node::Empty(_)) {
                    return Ok(empty());
                }
                let count = |t: &Option<TermOrVar>| -> Result<Option<u64>> {
                    match t {
                        None => Ok(None),
                        Some(TermOrVar::Const(Value::Int(n))) if *n >= 0 => Ok(Some(*n as u64)),
                        Some(other) => Err(invalid(format!("invalid skip/limit {other:?}"))),
                    }
                };
                let keys = self.keys(&o.keys)?;
                Node::OrderLimit(Box::new(input), keys, count(&o.skip)?, count(&o.limit)?)
            }
            Op::RowNumber(r) => {
                let input = self.plan(&r.input)?;
                if matches!(input, Node::Empty(_)) {
                    return Ok(empty());
                }
                let keys = self.keys(&r.order)?;
                Node::RowNumber(Box::new(input), r.partition.clone(), keys, r.var.clone())
            }
        })
    }

    fn keys(&mut self, ks: &[tm_ir::Key]) -> Result<Vec<PKey>> {
        ks.iter()
            .map(|k| {
                Ok(PKey {
                    expr: self.expr(&k.expr)?,
                    desc: k.descending,
                })
            })
            .collect()
    }

    fn triple(&mut self, t: &TriplePattern) -> Result<Option<Node>> {
        if let TermOrVar::Const(Value::Iri(iri)) = &t.p {
            if let Some(vp) = VirtualPred::from_iri(iri) {
                return self.virtual_pattern(t, vp);
            }
        }
        let Some(view) = self.view(&t.view)? else {
            return Ok(None);
        };
        let Some(s) = self.pterm(&t.s, Pos::Subject)? else {
            return Ok(None);
        };
        let Some(p) = self.pterm(&t.p, Pos::Predicate)? else {
            return Ok(None);
        };
        let Some(o) = self.pterm(&t.o, Pos::Object)? else {
            return Ok(None);
        };
        let volatile = t.include_volatile
            && t.view.tx == TxSel::Now
            && t.view.valid == ValidSel::Unfiltered
            && t.eid.is_none();
        if volatile {
            if let PTerm::Id(pid) = p {
                return Ok(Some(Node::Volatile(PVolatile { s, p: pid, o })));
            }
        }
        let canonical = self.sem.graph_set == GraphSet::SetOfTriples
            && t.eid.is_none()
            && !self.elide_all
            && match &p {
                PTerm::Id(pid) => self.is_multi(*pid)?,
                PTerm::Var(_) => true,
            };
        Ok(Some(Node::Triple(PTriple {
            s,
            p,
            o,
            eid: t.eid.clone(),
            view,
            iso_group: t.iso_group,
            canonical,
        })))
    }

    fn virtual_pattern(&mut self, t: &TriplePattern, vp: VirtualPred) -> Result<Option<Node>> {
        let Some(view) = self.view(&t.view)? else {
            return Ok(None);
        };
        let subject = match &t.s {
            TermOrVar::Var(v) => PTerm::Var(v.clone()),
            TermOrVar::Id(id) if id.tag_bits() == Tag::Stmt as u8 => PTerm::Id(*id),
            TermOrVar::Const(v) => match v.canonical() {
                Value::Stmt(e) => PTerm::Id(e.oid()),
                _ => return Ok(None),
            },
            TermOrVar::Param(p) => return Err(invalid(format!("unbound parameter `{p}`"))),
            TermOrVar::Id(_) => return Ok(None),
        };
        let object = match &t.o {
            TermOrVar::Var(v) => PObj::Var(v.clone()),
            TermOrVar::Id(id) => match vp.object_column_value(*id) {
                Some(c) => PObj::Column(c),
                None => return Ok(None),
            },
            TermOrVar::Const(v) => match self.encode(v)? {
                Enc::Id(id) => match vp.object_column_value(id) {
                    Some(c) => PObj::Column(c),
                    None => return Ok(None),
                },
                _ => return Ok(None),
            },
            TermOrVar::Param(p) => return Err(invalid(format!("unbound parameter `{p}`"))),
        };
        Ok(Some(Node::Virtual(PVirtual {
            subject,
            pred: vp,
            object,
            view,
        })))
    }

    fn konst(&mut self, v: &Value) -> Result<PExpr> {
        let value = v.canonical();
        let id = match self.encode(&value)? {
            Enc::Id(id) => Some(id),
            _ => match value {
                Value::Str(_) | Value::Iri(_) | Value::Double(_) | Value::Decimal(_) => None,
                _ => Some(self.synthetic_id(&value)),
            },
        };
        Ok(PExpr::Const(PConst { value, id }))
    }

    /// Plans an expression.
    pub fn expr(&mut self, e: &Expr) -> Result<PExpr> {
        let b = |s: &mut Self, x: &Expr| s.expr(x).map(Box::new);
        Ok(match e {
            Expr::Var(v) => PExpr::Var(v.clone()),
            Expr::Const(v) => self.konst(v)?,
            Expr::Param(p) => return Err(invalid(format!("unbound parameter `{p}`"))),
            Expr::Cmp(op, a, c) => PExpr::Cmp(*op, b(self, a)?, b(self, c)?),
            Expr::SameTerm(a, c) => PExpr::SameTerm(b(self, a)?, b(self, c)?),
            Expr::And(xs) => PExpr::And(self.exprs(xs)?),
            Expr::Or(xs) => PExpr::Or(self.exprs(xs)?),
            Expr::Not(a) => PExpr::Not(b(self, a)?),
            Expr::Bound(v) => PExpr::Bound(v.clone()),
            Expr::In(a, xs, n) => PExpr::In(b(self, a)?, self.exprs(xs)?, *n),
            Expr::Arith(op, a, c) => PExpr::Arith(*op, b(self, a)?, b(self, c)?),
            Expr::Neg(a) => PExpr::Neg(b(self, a)?),
            Expr::Coalesce(xs) => PExpr::Coalesce(self.exprs(xs)?),
            Expr::If(a, c, d) => PExpr::If(b(self, a)?, b(self, c)?, b(self, d)?),
            Expr::Func(f, xs) => PExpr::Func(*f, self.exprs(xs)?),
            Expr::Exists(op, neg) => match self.plan(op)? {
                Node::Empty(_) => PExpr::Bool(*neg),
                n => PExpr::Exists(Box::new(n), *neg),
            },
            Expr::Lookup(l) => {
                let subject = b(self, &l.subject)?;
                let pred = match &l.pred {
                    TermOrVar::Id(id) => Some(*id),
                    TermOrVar::Const(v) => match self.encode(v)? {
                        Enc::Id(id) if id.tag_bits() == Tag::Iri as u8 => Some(id),
                        _ => None,
                    },
                    other => return Err(invalid(format!("invalid lookup predicate {other:?}"))),
                };
                let view = self.view(&l.view)?;
                match (pred, view) {
                    (Some(pred), Some(view)) => PExpr::Lookup(PLookup {
                        subject,
                        pred,
                        view,
                        multi: l.multi,
                        volatile: l.include_volatile,
                    }),
                    _ => PExpr::Null,
                }
            }
            Expr::List(xs) => PExpr::List(self.exprs(xs)?),
        })
    }

    fn exprs(&mut self, xs: &[Expr]) -> Result<Vec<PExpr>> {
        xs.iter().map(|x| self.expr(x)).collect()
    }
}

fn pad_missing(n: Node, pad: Vec<Var>) -> Node {
    if pad.is_empty() {
        n
    } else {
        Node::PadMissing(Box::new(n), pad)
    }
}
