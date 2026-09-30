//! Graph patterns: lowering of `MATCH` patterns to the IR (design Decision 5) and
//! matching them against input rows. The IR carries the graph work (patterns, label
//! and property existence tests, isomorphism); `WHERE` runs here on the rows.

use std::collections::{HashMap, HashSet};

use tm_core::vocab::SYS;
use tm_core::{Eid, Value};
use tm_ir::vocab as irv;
use tm_ir::{Expr, Func, Op, TermOrVar, TriplePattern, Values, Var, View};

use super::access::{is_sys, stmt_of, tv, virtual_eid, RDF_TYPE, TEMPORAL};
use super::{Exec, Flags, Row};
use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::runner::Rows;
use crate::value::Val;
use crate::vocab::VocabExt;

/// How a variable of the pattern is read back.
#[derive(Clone, Debug)]
pub(crate) struct Bind {
    pub cypher: String,
    pub ir: String,
    pub is_rel: bool,
    /// Bound by a scan or generator: must be a plain node.
    pub generated: bool,
    pub new: bool,
    /// A variable-length relationship: the IR column holds the decoded path text.
    pub path_list: bool,
}

/// One element of a named path, in pattern order.
#[derive(Clone, Debug)]
pub(crate) enum PItem {
    Node(String),
    Rel(String),
    /// A variable-length segment: the column of its decoded path text.
    Var(String),
}

impl PItem {
    pub(crate) fn ir(&self) -> &str {
        match self {
            PItem::Node(s) | PItem::Rel(s) | PItem::Var(s) => s,
        }
    }
}

pub(crate) struct PathBind {
    pub name: String,
    pub items: Vec<PItem>,
}

/// A decoded variable-length segment of one result row.
struct Seg {
    nodes: Vec<Val>,
    rels: Vec<Val>,
    stored: Vec<Eid>,
}

/// A variable-length relationship waiting for its endpoints to be anchored.
struct Deferred {
    left: String,
    right: String,
    span: crate::span::Span,
}

/// A lowered `MATCH` clause.
pub(crate) struct Plan {
    pub op: Op,
    pub filter: Option<Expr>,
    pub seeds: Vec<(String, String)>,
    pub dynamic: Vec<(String, Expr2)>,
    pub binds: Vec<Bind>,
    pub paths: Vec<PathBind>,
    pub impossible: bool,
    pub stmt_irs: Vec<String>,
    /// Columns of variable-length segments (decoded path text).
    pub varlens: Vec<String>,
    /// Relationships must be pairwise distinct across the whole pattern.
    pub iso: bool,
}

/// A row-dependent property value: the AST expression, evaluated per input row.
pub(crate) type Expr2 = crate::ast::Expr;

type Chunk = (Vec<usize>, Vec<Vec<Option<TermOrVar>>>);

struct NodeInfo {
    ir: String,
    cypher: Option<String>,
    labels: Vec<Vec<String>>, // resolved alternatives
    stmt: bool,
    pred: bool,
    props: Vec<(String, PropVal)>,
    id: Option<Value>,
}

#[derive(Clone)]
enum PropVal {
    Const(Val),
    Dynamic(String), // ir var of the seed column
}

fn exists(op: Op) -> Expr {
    Expr::exists(op)
}

fn and(mut xs: Vec<Expr>) -> Expr {
    if xs.len() == 1 {
        xs.remove(0)
    } else {
        Expr::And(xs)
    }
}

fn or(mut xs: Vec<Expr>) -> Expr {
    if xs.len() == 1 {
        xs.remove(0)
    } else {
        Expr::Or(xs)
    }
}

fn is_special(n: &Name, what: &str) -> bool {
    n.text == what && !n.text.contains(':')
}

fn depends_on_row(e: &crate::ast::Expr) -> bool {
    use crate::ast::ExprKind as K;
    match &e.kind {
        K::Var(_) | K::Exists(_) => true,
        K::Lit(_) | K::Param(_) | K::CountStar => false,
        K::List(xs) => xs.iter().any(depends_on_row),
        K::Map(es) => es.iter().any(|(_, x)| depends_on_row(x)),
        K::Unary(_, x) | K::IsNull(x, _) | K::Prop(x, _) | K::HasLabels(x, _) => depends_on_row(x),
        K::Binary(_, a, b) | K::Index(a, b) => depends_on_row(a) || depends_on_row(b),
        K::Call { args, .. } => args.iter().any(depends_on_row),
        _ => true,
    }
}

impl Exec<'_> {
    fn ir_for(
        &mut self,
        ir_of: &mut HashMap<String, String>,
        name: &Option<Name>,
    ) -> (String, Option<String>) {
        match name {
            Some(n) => {
                if let Some(x) = ir_of.get(&n.text) {
                    return (x.clone(), Some(n.text.clone()));
                }
                let ir = self.fresh("x");
                ir_of.insert(n.text.clone(), ir.clone());
                (ir, Some(n.text.clone()))
            }
            None => (self.fresh("a"), None),
        }
    }

    fn fresh(&mut self, p: &str) -> String {
        let n = self.next_uid();
        format!("{p}{n}")
    }

    fn not_sys(p: Expr) -> Expr {
        Expr::not(Expr::Func(
            Func::StrStarts,
            vec![Expr::Func(Func::Str, vec![p]), Expr::val(Value::str(SYS))],
        ))
    }

    fn in_list(x: Expr, set: &HashSet<String>) -> Option<Expr> {
        if set.is_empty() {
            return None;
        }
        let mut v: Vec<&String> = set.iter().collect();
        v.sort();
        Some(Expr::In(
            Box::new(x),
            v.into_iter()
                .map(|i| Expr::val(Value::iri(i.clone())))
                .collect(),
            false,
        ))
    }

    /// The class filter of an object position for predicate variable `p`.
    fn rel_class(f: &Flags, p: Expr, o: Expr) -> Expr {
        let lit = Expr::not(Expr::Func(Func::IsLiteral, vec![o]));
        let ed = Self::in_list(p.clone(), &f.edge_false);
        let et = Self::in_list(p, &f.edge_true);
        let base = match ed {
            Some(ed) => Expr::And(vec![lit, Expr::not(ed)]),
            None => lit,
        };
        match et {
            Some(et) => Expr::Or(vec![et, base]),
            None => base,
        }
    }

    /// The existence test `x k v` under `view` for one property-map entry.
    fn prop_constraint(
        &mut self,
        subject: TermOrVar,
        key: &str,
        val: &PropVal,
        view: View,
        stmt_subject: bool,
        f: &Flags,
    ) -> Option<Expr> {
        let (iri, virt) = if stmt_subject && !key.contains(':') && TEMPORAL.contains(&key) {
            (
                match key {
                    "txAdded" => irv::TM_TX_ADDED,
                    "txRetracted" => irv::TM_TX_RETRACTED,
                    "validFrom" => irv::TM_VALID_FROM,
                    _ => irv::TM_VALID_TO,
                }
                .to_string(),
                true,
            )
        } else {
            (key.to_string(), false)
        };
        if !virt && f.edge_true.contains(&iri) {
            return None;
        }
        if let PropVal::Dynamic(pv) = val {
            // the row's candidate terms are seeded in `pv`; the shared variable
            // correlates the existence test with the row
            let op = Op::Triple(TriplePattern::new(
                subject,
                TermOrVar::iri(iri),
                TermOrVar::var(pv),
                view,
            ));
            return Some(exists(op));
        }
        let o = self.fresh("o");
        let mut op = Op::Triple(TriplePattern::new(
            subject,
            TermOrVar::iri(iri.clone()),
            TermOrVar::var(&o),
            view,
        ));
        if !virt && !f.edge_false.contains(&iri) {
            op = op.filter(Expr::Func(Func::IsLiteral, vec![Expr::var(&o)]));
        }
        let rhs = match val {
            PropVal::Const(v) => {
                let t = if virt && matches!(v, Val::Int(_)) {
                    match v {
                        Val::Int(i) => Value::Tx(tm_core::TxId(*i as u64)),
                        _ => return None,
                    }
                } else {
                    v.to_term()?
                };
                Expr::val(t)
            }
            PropVal::Dynamic(pv) => Expr::var(pv),
        };
        Some(exists(op.filter(Expr::eq(Expr::var(&o), rhs))))
    }

    /// Lowers a pattern for the rows' bound variables.
    pub(crate) fn lower_pattern(
        &mut self,
        pattern: &Pattern,
        bound: &HashSet<String>,
        mode: MatchModeExt,
        view: View,
    ) -> CResult<Plan> {
        let flags = self.flags(view)?;
        let mut ir_of: HashMap<String, String> = HashMap::new();
        let mut binds: Vec<Bind> = Vec::new();
        let mut infos: Vec<NodeInfo> = Vec::new();
        let mut ops: Vec<Op> = Vec::new();
        let mut filters: Vec<Expr> = Vec::new();
        let mut rel_eids: Vec<String> = Vec::new();
        let mut endpoint: HashSet<String> = HashSet::new();
        let mut dynamic: Vec<(String, Expr2)> = Vec::new();
        let mut paths: Vec<PathBind> = Vec::new();
        let mut varlens: Vec<String> = Vec::new();
        let mut deferred: Vec<Deferred> = Vec::new();
        let mut impossible = false;
        let mut eid_vars: HashSet<String> = HashSet::new();
        let mut stmt_irs: Vec<String> = Vec::new();

        for part in pattern {
            let mut node_irs: Vec<String> = Vec::new();
            let mut part_rels: Vec<PItem> = Vec::new();
            for n in &part.nodes {
                let (ir, cy) = self.ir_for(&mut ir_of, &n.var);
                let mut info = NodeInfo {
                    ir: ir.clone(),
                    cypher: cy.clone(),
                    labels: Vec::new(),
                    stmt: false,
                    pred: false,
                    props: Vec::new(),
                    id: None,
                };
                for g in &n.labels {
                    if g.len() == 1 && is_special(&g[0], "Statement") {
                        info.stmt = true;
                    } else if g.len() == 1 && is_special(&g[0], "Predicate") {
                        info.pred = true;
                    } else if g
                        .iter()
                        .any(|x| is_special(x, "Statement") || is_special(x, "Predicate"))
                    {
                        return Err(CypherError::unsupported(
                            "Statement/Predicate inside a label disjunction",
                            Some(g[0].span),
                        ));
                    } else {
                        let mut alts = Vec::new();
                        for a in g {
                            alts.push(self.vocab.resolve(a)?);
                        }
                        info.labels.push(alts);
                    }
                }
                if let Some(pm) = &n.props {
                    self.collect_props(
                        pm,
                        &mut info.props,
                        &mut info.id,
                        &mut dynamic,
                        &mut impossible,
                    )?;
                }
                node_irs.push(ir.clone());
                if let Some(c) = &cy {
                    if !binds.iter().any(|b| &b.cypher == c) {
                        binds.push(Bind {
                            cypher: c.clone(),
                            ir: ir.clone(),
                            is_rel: false,
                            generated: false,
                            path_list: false,
                            new: !bound.contains(c),
                        });
                    }
                } else {
                    binds.push(Bind {
                        cypher: String::new(),
                        ir: ir.clone(),
                        is_rel: false,
                        generated: false,
                        path_list: false,
                        new: true,
                    });
                }
                infos.push(info);
            }
            for (i, r) in part.rels.iter().enumerate() {
                if let Some(vl) = &r.var_len {
                    // a variable-length relationship is a path region
                    let (l, rr) = (node_irs[i].clone(), node_irs[i + 1].clone());
                    let pv = self.fresh("pp");
                    let op = self.lower_varlen(
                        part.shortest,
                        r,
                        vl,
                        (&l, &rr),
                        &pv,
                        &flags,
                        view,
                        &mut impossible,
                    )?;
                    ops.push(op);
                    binds.push(Bind {
                        cypher: r.var.as_ref().map(|v| v.text.clone()).unwrap_or_default(),
                        ir: pv.clone(),
                        is_rel: false,
                        generated: false,
                        path_list: true,
                        new: r.var.as_ref().is_none_or(|v| !bound.contains(&v.text)),
                    });
                    varlens.push(pv.clone());
                    part_rels.push(PItem::Var(pv));
                    deferred.push(Deferred {
                        left: l,
                        right: rr,
                        span: r.span,
                    });
                    continue;
                }
                let (a, b) = match r.dir {
                    Dir::Left => (node_irs[i + 1].clone(), node_irs[i].clone()),
                    _ => (node_irs[i].clone(), node_irs[i + 1].clone()),
                };
                let (e, ecy) = self.ir_for(&mut ir_of, &r.var);
                part_rels.push(PItem::Rel(e.clone()));
                eid_vars.insert(e.clone());
                endpoint.insert(a.clone());
                endpoint.insert(b.clone());
                if !rel_eids.contains(&e) {
                    rel_eids.push(e.clone());
                }
                if let Some(c) = &ecy {
                    if !binds.iter().any(|x| &x.cypher == c) {
                        binds.push(Bind {
                            cypher: c.clone(),
                            ir: e.clone(),
                            is_rel: true,
                            generated: false,
                            path_list: false,
                            new: !bound.contains(c),
                        });
                    }
                } else {
                    binds.push(Bind {
                        cypher: String::new(),
                        ir: e.clone(),
                        is_rel: true,
                        generated: false,
                        path_list: false,
                        new: true,
                    });
                }
                let rel_op = self.lower_rel(r, &a, &b, &e, &flags, view, &mut impossible)?;
                ops.push(rel_op);
                if let Some(pm) = &r.props {
                    let mut props = Vec::new();
                    let mut id = None;
                    self.collect_props(pm, &mut props, &mut id, &mut dynamic, &mut impossible)?;
                    for (k, v) in props {
                        let iri = if TEMPORAL.contains(&k.as_str()) {
                            k.clone()
                        } else {
                            k
                        };
                        match self.prop_constraint(TermOrVar::var(&e), &iri, &v, view, true, &flags)
                        {
                            Some(x) => filters.push(x),
                            None => impossible = true,
                        }
                    }
                }
            }
            let mut items_out: Vec<PItem> = Vec::new();
            for (i, nir) in node_irs.iter().enumerate() {
                items_out.push(PItem::Node(nir.clone()));
                if let Some(e) = part_rels.get(i) {
                    items_out.push(e.clone());
                }
            }
            if let Some(b) = &part.binding {
                paths.push(PathBind {
                    name: b.text.clone(),
                    items: items_out,
                });
            }
        }
        // a variable-length pattern needs a bound endpoint: seeded by an earlier clause,
        // constrained by an identity, label or property, or bound by another pattern
        // (the resolution repeats, because a path end can anchor the next path)
        let anchored = |info: &NodeInfo, endpoint: &HashSet<String>| -> bool {
            info.cypher.as_ref().is_some_and(|c| bound.contains(c))
                || info.id.is_some()
                || info.stmt
                || info.pred
                || !info.labels.is_empty()
                || !info.props.is_empty()
                || endpoint.contains(&info.ir)
        };
        let mut pending: Vec<&Deferred> = deferred.iter().collect();
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|d| {
                let is_anchored = |ir: &str, endpoint: &HashSet<String>| {
                    infos
                        .iter()
                        .filter(|i| i.ir == ir)
                        .any(|i| anchored(i, endpoint))
                };
                let (la, ra) = (
                    is_anchored(&d.left, &endpoint),
                    is_anchored(&d.right, &endpoint),
                );
                if la || ra {
                    // the far end is bound by the path itself: no scan generates it
                    for (ir, ok) in [(&d.left, la), (&d.right, ra)] {
                        if !ok {
                            endpoint.insert(ir.clone());
                            filters
                                .push(Expr::not(Expr::Func(Func::IsLiteral, vec![Expr::var(ir)])));
                        }
                    }
                    false
                } else {
                    true
                }
            });
            if pending.len() == before {
                return Err(CypherError::unsupported(
                    "path pattern with no bound endpoint (a variable-length or shortest-path \
                     pattern needs a bound start or end node)",
                    Some(pending[0].span),
                ));
            }
        }
        // node constraints and generators
        for info in &infos {
            let n = TermOrVar::var(&info.ir);
            let seeded = info.cypher.as_ref().is_some_and(|c| bound.contains(c));
            let bound_by_triple = endpoint.contains(&info.ir) || eid_vars.contains(&info.ir);
            #[allow(unused_assignments)]
            let mut generated_by: Option<Op> = None;
            let mut skip_label_group: Option<usize> = None;
            if !seeded && !bound_by_triple {
                if let Some(id) = &info.id {
                    generated_by = Some(Op::Values(Values {
                        vars: vec![Var::new(&info.ir)],
                        rows: vec![vec![Some(tv(id))]],
                    }));
                    // the identity must denote something in the store
                    let (p, o, s2) = (self.fresh("p"), self.fresh("o"), self.fresh("s"));
                    let present = if matches!(id, Value::Stmt(_)) {
                        exists(Op::Triple(
                            TriplePattern::new(
                                TermOrVar::var(&s2),
                                TermOrVar::var(&p),
                                TermOrVar::var(&o),
                                view,
                            )
                            .with_eid(&info.ir),
                        ))
                    } else {
                        or(vec![
                            exists(Op::Triple(TriplePattern::new(
                                n.clone(),
                                TermOrVar::var(&p),
                                TermOrVar::var(&o),
                                view,
                            ))),
                            exists(Op::Triple(TriplePattern::new(
                                TermOrVar::var(&s2),
                                TermOrVar::var(&p),
                                n.clone(),
                                view,
                            ))),
                        ])
                    };
                    filters.push(present);
                } else if info.stmt {
                    let (p, o) = (self.fresh("p"), self.fresh("o"));
                    generated_by = Some(
                        Op::Triple(
                            TriplePattern::new(
                                TermOrVar::var(&self.fresh("s")),
                                TermOrVar::var(&p),
                                TermOrVar::var(&o),
                                view,
                            )
                            .with_eid(&info.ir),
                        )
                        .filter(Self::not_sys(Expr::var(&p)))
                        .project_distinct(&[&info.ir]),
                    );
                } else if info.pred {
                    generated_by = Some(self.predicate_gen(&info.ir, view));
                } else if let Some(gi) = info.labels.iter().position(|_| true) {
                    let alts = &info.labels[gi];
                    let branches: Vec<Op> = alts
                        .iter()
                        .map(|a| {
                            Op::Triple(TriplePattern::new(
                                n.clone(),
                                TermOrVar::iri(RDF_TYPE),
                                TermOrVar::iri(a.clone()),
                                view,
                            ))
                            .project_distinct(&[&info.ir])
                        })
                        .collect();
                    generated_by = Some(if branches.len() == 1 {
                        branches.into_iter().next().unwrap()
                    } else {
                        Op::union(branches).project_distinct(&[&info.ir])
                    });
                    skip_label_group = Some(gi);
                } else {
                    generated_by = Some(self.scan_gen(&info.ir, &flags, view));
                }
                if let Some(g) = generated_by.take() {
                    ops.push(g);
                    for b in binds.iter_mut() {
                        if b.ir == info.ir {
                            b.generated = !info.stmt && info.id.is_none();
                        }
                    }
                }
            }
            if let Some(id) = &info.id {
                if seeded || bound_by_triple {
                    // identity constraint on a bound variable
                    match id {
                        Value::Stmt(e) => filters
                            .push(Expr::eq(Expr::var(&info.ir), Expr::Const(Value::Stmt(*e)))),
                        other => {
                            filters.push(Expr::eq(Expr::var(&info.ir), Expr::val(other.clone())))
                        }
                    }
                }
            }
            if info.stmt && (seeded || bound_by_triple) {
                stmt_irs.push(info.ir.clone());
            }
            if info.pred && (seeded || bound_by_triple) {
                let ex = self.pred_exists(&info.ir, view);
                filters.push(or(ex));
            }
            for (gi, alts) in info.labels.iter().enumerate() {
                if Some(gi) == skip_label_group {
                    continue;
                }
                let ex: Vec<Expr> = alts
                    .iter()
                    .map(|a| {
                        exists(Op::Triple(TriplePattern::new(
                            n.clone(),
                            TermOrVar::iri(RDF_TYPE),
                            TermOrVar::iri(a.clone()),
                            view,
                        )))
                    })
                    .collect();
                filters.push(or(ex));
            }
            for (k, v) in &info.props {
                match self.prop_constraint(n.clone(), k, v, view, false, &flags) {
                    Some(x) => filters.push(x),
                    None => impossible = true,
                }
            }
        }
        // isomorphism among this clause's relationship positions
        if mode == MatchModeExt::Default {
            for i in 0..rel_eids.len() {
                for j in i + 1..rel_eids.len() {
                    filters.push(Expr::ne(Expr::var(&rel_eids[i]), Expr::var(&rel_eids[j])));
                }
            }
        }
        // seeds: cypher variables that are bound in the input rows
        let mut seeds: Vec<(String, String)> = Vec::new();
        for (c, ir) in &ir_of {
            if bound.contains(c) {
                seeds.push((c.clone(), ir.clone()));
            }
        }
        seeds.sort();
        let op = if ops.is_empty() {
            Op::unit()
        } else {
            Op::join(ops)
        };
        let filter = if filters.is_empty() {
            None
        } else {
            Some(and(filters))
        };
        Ok(Plan {
            op,
            filter,
            seeds,
            dynamic,
            binds,
            paths,
            impossible,
            stmt_irs,
            iso: mode == MatchModeExt::Default && !varlens.is_empty(),
            varlens,
        })
    }

    fn collect_props(
        &mut self,
        pm: &crate::ast::Expr,
        out: &mut Vec<(String, PropVal)>,
        id: &mut Option<Value>,
        dynamic: &mut Vec<(String, Expr2)>,
        impossible: &mut bool,
    ) -> CResult<()> {
        let ExprKind::Map(entries) = &pm.kind else {
            // a parameter map: evaluate now
            let v = self.eval(pm, &Row::new())?;
            if let Val::Map(m) = v {
                for (k, x) in m {
                    self.push_prop_val(&k, false, x, out, id, impossible)?;
                }
                return Ok(());
            }
            return Err(CypherError::parse(pm.span, "a property map is expected"));
        };
        for (k, e) in entries {
            if depends_on_row(e) {
                let pv = self.fresh("pv");
                dynamic.push((pv.clone(), e.clone()));
                if k.text == "@id" && k.escaped {
                    // dynamic @id: resolved per row
                    return Err(CypherError::unsupported(
                        "a row-dependent @id in MATCH",
                        Some(k.span),
                    ));
                }
                let iri = self.vocab.resolve(k)?;
                out.push((iri, PropVal::Dynamic(pv)));
            } else {
                let v = self.eval(e, &Row::new())?;
                self.push_prop_val(&k.text, k.escaped, v, out, id, impossible)
                    .map_err(|err| match err {
                        CypherError::Parse { msg, .. } => CypherError::parse(k.span, msg),
                        other => other,
                    })?;
                if !(k.text == "@id" && k.escaped) {
                    // re-resolve the key with the vocabulary (push_prop_val used raw text)
                    if let Some(last) = out.last_mut() {
                        last.0 = self.vocab.resolve(k)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn push_prop_val(
        &mut self,
        key: &str,
        escaped: bool,
        v: Val,
        out: &mut Vec<(String, PropVal)>,
        id: &mut Option<Value>,
        impossible: &mut bool,
    ) -> CResult<()> {
        if key == "@id" && escaped {
            match &v {
                Val::Str(s) => {
                    *id = Some(self.vocab.resolve_id(s).map_err(CypherError::eval)?);
                }
                Val::Null => *impossible = true,
                _ => return Err(CypherError::eval("@id must be a string")),
            }
            return Ok(());
        }
        match &v {
            Val::Null | Val::List(_) | Val::Map(_) => *impossible = true,
            _ => out.push((key.to_string(), PropVal::Const(v))),
        }
        Ok(())
    }

    fn predicate_gen(&mut self, n: &str, view: View) -> Op {
        let branches: Vec<Op> = tm_core::vocab::SCHEMA_FLAGS
            .iter()
            .map(|f| {
                Op::Triple(TriplePattern::new(
                    TermOrVar::var(n),
                    TermOrVar::iri(*f),
                    TermOrVar::var(&format!("{n}_f")),
                    view,
                ))
                .project_distinct(&[n])
            })
            .collect();
        Op::union(branches).project_distinct(&[n])
    }

    fn pred_exists(&mut self, n: &str, view: View) -> Vec<Expr> {
        tm_core::vocab::SCHEMA_FLAGS
            .iter()
            .map(|f| {
                exists(Op::Triple(TriplePattern::new(
                    TermOrVar::var(n),
                    TermOrVar::iri(*f),
                    TermOrVar::var(&self.fresh("f")),
                    view,
                )))
            })
            .collect()
    }

    /// The node scan (C4): subjects of visible non-`sys:` statements and objects of
    /// visible relationship statements.
    fn scan_gen(&mut self, n: &str, flags: &Flags, view: View) -> Op {
        let (p1, o1) = (self.fresh("p"), self.fresh("o"));
        let a = Op::Triple(TriplePattern::new(
            TermOrVar::var(n),
            TermOrVar::var(&p1),
            TermOrVar::var(&o1),
            view,
        ))
        .filter(Self::not_sys(Expr::var(&p1)))
        .project_distinct(&[n]);
        let (s2, p2) = (self.fresh("s"), self.fresh("p"));
        let mut conds = vec![
            Self::not_sys(Expr::var(&p2)),
            Expr::ne(Expr::var(&p2), Expr::val(Value::iri(RDF_TYPE))),
            Self::rel_class(flags, Expr::var(&p2), Expr::var(n)),
        ];
        let b = Op::Triple(TriplePattern::new(
            TermOrVar::var(&s2),
            TermOrVar::var(&p2),
            TermOrVar::var(n),
            view,
        ))
        .filter(and(std::mem::take(&mut conds)))
        .project_distinct(&[n]);
        Op::union(vec![a, b]).project_distinct(&[n])
    }

    /// A variable-length or shortest-path relationship as a path region
    /// (`path-lowering`): `TRAIL` for `*`, `ANY_SHORTEST` / `ALL_SHORTEST` for
    /// `shortestPath` / `allShortestPaths`. An unbounded upper limit is the
    /// database's hop cap, applied as a search depth bound.
    #[allow(clippy::too_many_arguments)]
    // @lat: [[query#Physical Planning#Path Engine#Path Lowering]]
    fn lower_varlen(
        &mut self,
        shortest: Option<bool>,
        r: &RelPat,
        vl: &VarLen,
        (left, right): (&str, &str),
        pv: &str,
        flags: &Flags,
        view: View,
        impossible: &mut bool,
    ) -> CResult<Op> {
        let mode = match shortest {
            None => tm_ir::PathMode::Trail,
            Some(false) => tm_ir::PathMode::AnyShortest,
            Some(true) => tm_ir::PathMode::AllShortest,
        };
        if shortest.is_some() && vl.min > 1 {
            return Err(CypherError::unsupported(
                "shortest-path minimum length (Cypher allows 0 or 1)",
                Some(vl.span),
            ));
        }
        let mut atoms: Vec<tm_ir::PathExpr> = Vec::new();
        if r.types.is_empty() {
            atoms.push(tm_ir::PathExpr::iri(irv::SYS_ANY_RELATIONSHIP));
        } else {
            let mut seen: Vec<String> = Vec::new();
            for t in &r.types {
                let iri = self.vocab.resolve(t)?;
                if iri == RDF_TYPE || flags.edge_false.contains(&iri) || seen.contains(&iri) {
                    continue;
                }
                seen.push(iri.clone());
                atoms.push(tm_ir::PathExpr::iri(iri));
            }
        }
        let empty_range = vl.max.is_some_and(|m| m < vl.min);
        if atoms.is_empty() || empty_range {
            *impossible = true;
            return Ok(Op::Values(Values {
                vars: vec![Var::new(left), Var::new(pv), Var::new(right)],
                rows: Vec::new(),
            }));
        }
        let one = if atoms.len() == 1 {
            atoms.remove(0)
        } else {
            tm_ir::PathExpr::Alt(atoms)
        };
        let step = match r.dir {
            Dir::Right => one,
            Dir::Left => one.inverse(),
            Dir::Either => tm_ir::PathExpr::Alt(vec![one.clone(), one.inverse()]),
        };
        let cap = self.runner.path_max_hops();
        Ok(Op::Path(tm_ir::PathPattern {
            start: TermOrVar::var(left),
            end: TermOrVar::var(right),
            path: tm_ir::PathExpr::Repeat {
                inner: Box::new(step),
                min: vl.min,
                max: vl.max,
            },
            mode,
            max_hops: Some(vl.max.unwrap_or(cap)),
            bind_path: Some(Var::new(pv)),
            view,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_rel(
        &mut self,
        r: &RelPat,
        a: &str,
        b: &str,
        e: &str,
        flags: &Flags,
        view: View,
        impossible: &mut bool,
    ) -> CResult<Op> {
        let orientations: Vec<(String, String)> = match r.dir {
            Dir::Either if a != b => vec![
                (a.to_string(), b.to_string()),
                (b.to_string(), a.to_string()),
            ],
            _ => vec![(a.to_string(), b.to_string())],
        };
        let mut branches: Vec<Op> = Vec::new();
        for (oi, (s, o)) in orientations.iter().enumerate() {
            let self_loop_guard = |op: Op| -> Op {
                if oi == 1 {
                    op.filter(Expr::not(Expr::SameTerm(
                        Box::new(Expr::var(s)),
                        Box::new(Expr::var(o)),
                    )))
                } else {
                    op
                }
            };
            if r.types.is_empty() {
                let p = self.fresh("p");
                let op = Op::Triple(
                    TriplePattern::new(
                        TermOrVar::var(s),
                        TermOrVar::var(&p),
                        TermOrVar::var(o),
                        view,
                    )
                    .with_eid(e),
                )
                .filter(Expr::And(vec![
                    Self::not_sys(Expr::var(&p)),
                    Expr::ne(Expr::var(&p), Expr::val(Value::iri(RDF_TYPE))),
                    Self::rel_class(flags, Expr::var(&p), Expr::var(o)),
                ]));
                branches.push(self_loop_guard(op).project(&[s.as_str(), e, o.as_str()]));
            } else {
                let mut seen_types: Vec<String> = Vec::new();
                for t in &r.types {
                    let iri = self.vocab.resolve(t)?;
                    if iri == RDF_TYPE
                        || flags.edge_false.contains(&iri)
                        || seen_types.contains(&iri)
                    {
                        continue;
                    }
                    seen_types.push(iri.clone());
                    let mut op = Op::Triple(
                        TriplePattern::new(
                            TermOrVar::var(s),
                            TermOrVar::iri(iri.clone()),
                            TermOrVar::var(o),
                            view,
                        )
                        .with_eid(e),
                    );
                    if !flags.edge_true.contains(&iri) {
                        op = op.filter(Expr::not(Expr::Func(Func::IsLiteral, vec![Expr::var(o)])));
                    }
                    branches.push(self_loop_guard(op).project(&[s.as_str(), e, o.as_str()]));
                }
            }
        }
        if branches.is_empty() {
            *impossible = true;
            // a well-formed but empty operator
            return Ok(Op::Values(Values {
                vars: vec![Var::new(a), Var::new(e), Var::new(b)],
                rows: Vec::new(),
            }));
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Op::union(branches)
        })
    }
}

/// The stored terms a Cypher value may equal (numbers compare by value).
fn candidates(v: &Val) -> Vec<Value> {
    match v {
        Val::Int(i) => {
            let mut out = vec![Value::Int(*i).canonical()];
            if i.unsigned_abs() < (1u64 << 53) {
                out.push(Value::Double(*i as f64));
            }
            out
        }
        Val::Float(x) => {
            let mut out = vec![Value::Double(*x)];
            if x.is_finite() && *x == x.trunc() && x.abs() < 9.0e15 {
                out.push(Value::Int(*x as i64));
            }
            out
        }
        other => other.to_term().into_iter().collect(),
    }
}

fn seed_cell(v: &Val) -> Option<TermOrVar> {
    match v {
        Val::Node(x) => Some(tv(x)),
        Val::Rel(e) => Some(TermOrVar::Id(e.oid())),
        _ => None,
    }
}

impl Exec<'_> {
    /// Matches `pattern` against every input row (`OPTIONAL` keeps unmatched rows).
    pub(crate) fn exec_match(
        &mut self,
        rows: Vec<Row>,
        pattern: &Pattern,
        where_: Option<&crate::ast::Expr>,
        optional: bool,
        mode: MatchModeExt,
    ) -> CResult<Vec<Row>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let view = self.view();
        let bound: HashSet<String> = rows[0].keys().cloned().collect();
        let plan = self.lower_pattern(pattern, &bound, mode, view)?;
        let new_vars: Vec<&Bind> = plan
            .binds
            .iter()
            .filter(|b| b.new && !b.cypher.is_empty())
            .collect();
        let mut out: Vec<Row> = Vec::new();
        let mut matched: Vec<Vec<Row>> = vec![Vec::new(); rows.len()];
        if !plan.impossible {
            // build per-row seeds
            let mut valid_rows: Vec<usize> = Vec::new();
            let mut cells: Vec<Vec<Option<TermOrVar>>> = Vec::new();
            for (i, r) in rows.iter().enumerate() {
                let mut cs: Vec<Option<TermOrVar>> =
                    vec![Some(TermOrVar::Const(Value::Int(i as i64)))];
                let mut ok = true;
                for (c, _) in &plan.seeds {
                    match r.get(c).and_then(seed_cell) {
                        Some(x) => cs.push(Some(x)),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                let mut combos: Vec<Vec<Option<TermOrVar>>> = vec![cs];
                if ok {
                    for (_, e) in &plan.dynamic {
                        let v = self.eval(e, r)?;
                        let cand = candidates(&v);
                        if cand.is_empty() {
                            ok = false;
                            break;
                        }
                        let mut next = Vec::new();
                        for base in &combos {
                            for c in &cand {
                                let mut x = base.clone();
                                x.push(Some(TermOrVar::Const(c.clone())));
                                next.push(x);
                            }
                        }
                        combos = next;
                    }
                }
                if ok {
                    for c in combos {
                        valid_rows.push(i);
                        cells.push(c);
                    }
                }
            }
            let need_seed = !plan.seeds.is_empty() || !plan.dynamic.is_empty();
            let mut cols: Vec<String> = Vec::new();
            for b in &plan.binds {
                if !cols.contains(&b.ir) {
                    cols.push(b.ir.clone());
                }
            }
            for p in &plan.paths {
                for it in &p.items {
                    if !cols.iter().any(|c| c == it.ir()) {
                        cols.push(it.ir().to_string());
                    }
                }
            }
            for ir in &plan.stmt_irs {
                if !cols.contains(ir) {
                    cols.push(ir.clone());
                }
            }
            let chunks: Vec<Chunk> = if need_seed {
                let mut v = Vec::new();
                let mut idx = valid_rows.chunks(256);
                let mut cc = cells.chunks(256);
                while let (Some(i), Some(c)) = (idx.next(), cc.next()) {
                    v.push((i.to_vec(), c.to_vec()));
                }
                v
            } else {
                vec![(valid_rows.clone(), cells.clone())]
            };
            for (_, cs) in chunks {
                let op = if need_seed {
                    let mut vars = vec![Var::new("__row")];
                    for (_, ir) in &plan.seeds {
                        vars.push(Var::new(ir));
                    }
                    for (pv, _) in &plan.dynamic {
                        vars.push(Var::new(pv));
                    }
                    let vals = Op::Values(Values { vars, rows: cs });
                    let mut joined = Op::join(vec![vals, plan.op.clone()]);
                    if let Some(f) = &plan.filter {
                        joined = joined.filter(f.clone());
                    }
                    let mut proj: Vec<&str> = vec!["__row"];
                    proj.extend(cols.iter().map(String::as_str));
                    if plan.dynamic.is_empty() {
                        joined.project(&proj)
                    } else {
                        joined.project_distinct(&proj)
                    }
                } else {
                    let proj: Vec<&str> = cols.iter().map(String::as_str).collect();
                    let mut base = plan.op.clone();
                    if let Some(f) = &plan.filter {
                        base = base.filter(f.clone());
                    }
                    base.project(&proj)
                };
                let res = self.run_op(op)?;
                self.absorb(&plan, &res, &rows, &mut matched, need_seed)?;
            }
        }
        if !plan.impossible && plan.seeds.is_empty() && plan.dynamic.is_empty() {
            // a pattern independent of the input rows: every row gets the same matches
            let first = matched[0].clone();
            for slot in matched.iter_mut().skip(1) {
                *slot = first.clone();
            }
        }
        for (i, r) in rows.iter().enumerate() {
            let mut any = false;
            for m in std::mem::take(&mut matched[i]) {
                let mut row = r.clone();
                for (k, v) in m {
                    row.insert(k, v);
                }
                if let Some(w) = where_ {
                    if !matches!(self.eval(w, &row)?, Val::Bool(true)) {
                        continue;
                    }
                }
                any = true;
                out.push(row);
            }
            if !any && optional {
                let mut row = r.clone();
                for b in &new_vars {
                    row.insert(b.cypher.clone(), Val::Null);
                }
                for p in &plan.paths {
                    row.insert(p.name.clone(), Val::Null);
                }
                out.push(row);
            }
        }
        Ok(out)
    }

    /// Reads the decoded path text of a variable-length segment: its nodes, its
    /// relationships (virtual layer hops as synthetic relationships) and the stored
    /// eids among them.
    fn parse_seg(&self, text: &str) -> Option<Seg> {
        let j: serde_json::Value = serde_json::from_str(text).ok()?;
        let entity = |s: &str| self.vocab.resolve_id(s).ok();
        let nodes = j["nodes"]
            .as_array()?
            .iter()
            .map(|n| entity(n.as_str()?).map(|v| Val::from_entity(&v)))
            .collect::<Option<Vec<_>>>()?;
        let mut rels = Vec::new();
        let mut stored = Vec::new();
        for e in j["edges"].as_array()? {
            let Some(Value::Stmt(eid)) = entity(e["eid"].as_str()?) else {
                return None;
            };
            let kind = match e["p"].as_str()? {
                irv::SYS_SUBJECT => Some(0),
                irv::SYS_OBJECT => Some(1),
                irv::SYS_PREDICATE => Some(2),
                _ => None,
            };
            match kind {
                Some(k) => rels.push(Val::Rel(virtual_eid(eid, k))),
                None => {
                    stored.push(eid);
                    rels.push(Val::Rel(eid));
                }
            }
        }
        Some(Seg {
            nodes,
            rels,
            stored,
        })
    }

    fn absorb(
        &mut self,
        plan: &Plan,
        res: &Rows,
        rows: &[Row],
        matched: &mut [Vec<Row>],
        need_seed: bool,
    ) -> CResult<()> {
        let row_col = res.col("__row");
        'next: for r in &res.rows {
            let idx = if need_seed {
                match row_col.and_then(|c| r[c].as_ref()) {
                    Some(Value::Int(i)) => *i as usize,
                    _ => continue,
                }
            } else {
                0
            };
            let mut m = Row::new();
            let mut by_ir: HashMap<&str, Val> = HashMap::new();
            let mut segs: HashMap<&str, Seg> = HashMap::new();
            for pv in &plan.varlens {
                let Some(ci) = res.col(pv) else {
                    continue 'next;
                };
                let Some(Value::Str(text)) = &r[ci] else {
                    continue 'next;
                };
                let Some(seg) = self.parse_seg(text) else {
                    continue 'next;
                };
                segs.insert(pv.as_str(), seg);
            }
            for b in &plan.binds {
                if b.path_list {
                    let Some(seg) = segs.get(b.ir.as_str()) else {
                        continue 'next;
                    };
                    let v = Val::List(seg.rels.clone());
                    if !b.cypher.is_empty() && b.new {
                        m.insert(b.cypher.clone(), v.clone());
                    }
                    by_ir.insert(b.ir.as_str(), v);
                    continue;
                }
                let Some(ci) = res.col(&b.ir) else { continue };
                let Some(cell) = &r[ci] else { continue 'next };
                let v = if b.is_rel {
                    match cell {
                        Value::Stmt(e) => Val::Rel(*e),
                        _ => continue 'next,
                    }
                } else {
                    if b.generated
                        && !matches!(cell, Value::Iri(_) | Value::Node(_) | Value::BNode(_))
                    {
                        continue 'next;
                    }
                    Val::from_entity(cell)
                };
                if !b.cypher.is_empty() && b.new {
                    m.insert(b.cypher.clone(), v.clone());
                }
                by_ir.insert(b.ir.as_str(), v);
            }
            if plan.iso {
                // relationships are pairwise distinct across fixed and variable-length
                // positions (virtual layer hops are not relationships)
                let mut used: HashSet<Eid> = HashSet::new();
                let fixed = plan.binds.iter().filter(|b| b.is_rel).filter_map(|b| {
                    match by_ir.get(b.ir.as_str()) {
                        Some(Val::Rel(e)) => Some(*e),
                        _ => None,
                    }
                });
                let in_paths = segs.values().flat_map(|s| s.stored.iter().copied());
                if !fixed.chain(in_paths).all(|e| used.insert(e)) {
                    continue 'next;
                }
            }
            // a bound variable keeps the form of its first binding
            for p in &plan.paths {
                let mut items = Vec::new();
                let mut skip_node = false;
                for it in &p.items {
                    let ir = it.ir();
                    if let PItem::Var(pv) = it {
                        let seg = &segs[pv.as_str()];
                        for (i, rel) in seg.rels.iter().enumerate() {
                            items.push(rel.clone());
                            if i + 1 < seg.rels.len() {
                                items.push(seg.nodes[i + 1].clone());
                            }
                        }
                        // a zero-length segment: its two ends are one node
                        skip_node = seg.rels.is_empty();
                        continue;
                    }
                    if matches!(it, PItem::Node(_)) && std::mem::take(&mut skip_node) {
                        continue;
                    }
                    let is_rel = matches!(it, PItem::Rel(_));
                    let v = match by_ir.get(ir) {
                        Some(v) => v.clone(),
                        None => {
                            let Some(ci) = res.col(ir) else {
                                continue 'next;
                            };
                            let Some(cell) = &r[ci] else { continue 'next };
                            if is_rel {
                                match cell {
                                    Value::Stmt(e) => Val::Rel(*e),
                                    _ => continue 'next,
                                }
                            } else {
                                Val::from_entity(cell)
                            }
                        }
                    };
                    items.push(v);
                }
                m.insert(p.name.clone(), Val::Path(items));
            }
            for ir in &plan.stmt_irs {
                if let Some(ci) = res.col(ir) {
                    if !matches!(r[ci], Some(Value::Stmt(_))) {
                        continue 'next;
                    }
                }
            }
            let _ = rows;
            matched[idx].push(m);
        }
        let _ = stmt_of;
        let _ = is_sys;
        Ok(())
    }
}
