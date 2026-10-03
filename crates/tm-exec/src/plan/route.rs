//! Region routing (design D7): path patterns go to the native path operator (the
//! `tm_path` table-valued function), everything else to SQL. A cyclic BGP stays
//! in SQL while LFTJ is disabled or has no operator (M4).

use std::collections::BTreeSet;

use tm_core::Result;
use tm_ir::{Expr, IrQuery, Op, PathPattern, TermOrVar, Var, VarSet};

use crate::error::{unsupported, NO_PATH_OPERATOR};
use crate::native::{OperatorRegistry, PlannerOptions};
use crate::plan::analyze::is_cyclic;
use crate::result::RouteNote;

/// How a path is called.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// From its bound start.
    Forward,
    /// From its bound end, with the inverse path.
    Inverted,
}

fn add_term(t: &TermOrVar, out: &mut VarSet) {
    if let TermOrVar::Var(v) = t {
        out.insert(v.clone());
    }
}

fn expr_ops<'a>(e: &'a Expr, out: &mut Vec<&'a Op>) {
    match e {
        Expr::Exists(op, _) => out.push(op),
        Expr::Cmp(_, a, b) | Expr::SameTerm(a, b) | Expr::Arith(_, a, b) => {
            expr_ops(a, out);
            expr_ops(b, out);
        }
        Expr::And(xs) | Expr::Or(xs) | Expr::Coalesce(xs) | Expr::List(xs) | Expr::Func(_, xs) => {
            xs.iter().for_each(|x| expr_ops(x, out))
        }
        Expr::Not(a) | Expr::Neg(a) => expr_ops(a, out),
        Expr::In(a, xs, _) => {
            expr_ops(a, out);
            xs.iter().for_each(|x| expr_ops(x, out));
        }
        Expr::If(a, b, c) => {
            expr_ops(a, out);
            expr_ops(b, out);
            expr_ops(c, out);
        }
        Expr::Lookup(l) => expr_ops(&l.subject, out),
        Expr::Var(_) | Expr::Const(_) | Expr::Param(_) | Expr::Bound(_) => {}
    }
}

/// Operators nested in the expressions of `op` (existence tests).
fn nested_ops(op: &Op) -> Vec<&Op> {
    let mut out = Vec::new();
    match op {
        Op::Filter(f) => expr_ops(&f.cond, &mut out),
        Op::LeftJoin(l) => {
            if let Some(c) = &l.cond {
                expr_ops(c, &mut out)
            }
        }
        Op::Extend(e) => expr_ops(&e.expr, &mut out),
        Op::Unnest(u) => expr_ops(&u.list, &mut out),
        Op::OrderLimit(o) => o.keys.iter().for_each(|k| expr_ops(&k.expr, &mut out)),
        _ => {}
    }
    out
}

/// The `Unsupported` feature of a time-respecting path whose start is not bound.
pub const TIME_RESPECTING_NEEDS_START: &str =
    "time-respecting path with no bound start (a journey runs forward from its start)";

/// Every variable bound by an operator other than a path pattern.
pub fn bound_by_non_paths(op: &Op) -> VarSet {
    let mut out = VarSet::new();
    fn walk(op: &Op, out: &mut VarSet) {
        match op {
            Op::Triple(t) => {
                add_term(&t.s, out);
                add_term(&t.p, out);
                add_term(&t.o, out);
                if let Some(e) = &t.eid {
                    out.insert(e.clone());
                }
            }
            Op::Values(v) => out.extend(v.vars.iter().cloned()),
            Op::Text(t) => {
                out.insert(t.eid.clone());
                for v in [&t.score, &t.rank, &t.confidence].into_iter().flatten() {
                    out.insert(v.clone());
                }
            }
            Op::Unnest(u) => {
                out.insert(u.var.clone());
            }
            Op::Extend(e) => {
                out.insert(e.var.clone());
            }
            Op::Aggregate(a) => out.extend(a.aggs.iter().map(|g| g.var.clone())),
            Op::RowNumber(r) => {
                out.insert(r.var.clone());
            }
            _ => {}
        }
        for c in op.children() {
            walk(c, out);
        }
        for n in nested_ops(op) {
            walk(n, out);
        }
    }
    walk(op, &mut out);
    out
}

/// The variables bound before path `skip` runs: those of [`bound_by_non_paths`]
/// plus the far endpoint (and path and graph variables) of every *other* path
/// whose one endpoint is bound, repeated until nothing changes (a path may start
/// where another ends).
pub fn bound_before(base: &VarSet, all: &[PathPattern], skip: &PathPattern) -> VarSet {
    let mut bound = base.clone();
    loop {
        let mut changed = false;
        for p in all.iter().filter(|p| *p != skip) {
            if endpoint_bound(&p.start, &bound) || endpoint_bound(&p.end, &bound) {
                for t in [&p.start, &p.end] {
                    if let TermOrVar::Var(v) = t {
                        changed |= bound.insert(v.clone());
                    }
                }
                if let Some(b) = &p.bind_path {
                    changed |= bound.insert(b.clone());
                }
                if let Some(a) = p.time_respecting.as_ref().and_then(|t| t.arrival.as_ref()) {
                    changed |= bound.insert(a.clone());
                }
                // a path under an unbound graph variable enumerates the graphs
                if let tm_ir::GraphSel::Var(g) = &p.graph {
                    changed |= bound.insert(g.clone());
                }
            }
        }
        if !changed {
            return bound;
        }
    }
}

/// Every path pattern of the tree (existence tests included).
pub fn path_patterns(op: &Op) -> Vec<PathPattern> {
    let mut ps = Vec::new();
    paths(op, &mut ps);
    ps.into_iter().cloned().collect()
}

fn endpoint_bound(t: &TermOrVar, bound: &VarSet) -> bool {
    match t {
        TermOrVar::Var(v) => bound.contains(v),
        _ => true,
    }
}

/// Orients a path: from its bound start, else from its bound end (inverse path);
/// no bound endpoint is `Unsupported`. A time-respecting path runs forward only (a
/// journey is not the inverse journey read backwards), so it needs a bound start.
// @lat: [[query#Physical Planning#Path Engine#Path Lowering]]
pub fn orient(p: &PathPattern, bound: &VarSet) -> Result<Orientation> {
    if endpoint_bound(&p.start, bound) {
        Ok(Orientation::Forward)
    } else if p.time_respecting.is_some() {
        Err(unsupported(TIME_RESPECTING_NEEDS_START))
    } else if endpoint_bound(&p.end, bound) {
        Ok(Orientation::Inverted)
    } else {
        Err(unsupported(
            "path pattern with no bound endpoint (a recursive path needs a bound start or end)",
        ))
    }
}

fn paths<'a>(op: &'a Op, out: &mut Vec<&'a PathPattern>) {
    if let Op::Path(p) = op {
        out.push(p);
    }
    for c in op.children() {
        paths(c, out);
    }
    for n in nested_ops(op) {
        paths(n, out);
    }
}

/// Checks, before any SQL runs, that every path pattern can be routed: a path
/// operator is registered and each path has a bound endpoint.
pub fn precheck(q: &IrQuery, reg: &OperatorRegistry) -> Result<()> {
    let mut ps = Vec::new();
    paths(&q.root, &mut ps);
    if ps.is_empty() {
        return Ok(());
    }
    if reg.path().is_none() {
        return Err(unsupported(NO_PATH_OPERATOR));
    }
    let base = bound_by_non_paths(&q.root);
    let all = path_patterns(&q.root);
    for p in &all {
        orient(p, &bound_before(&base, &all, p))?;
    }
    Ok(())
}

/// The route of a basic graph pattern given as the variable sets of its triple
/// patterns: always SQL in M1, with a note when it is cyclic.
pub fn route_bgp(
    edges: &[BTreeSet<Var>],
    opts: &PlannerOptions,
    reg: &OperatorRegistry,
) -> RouteNote {
    if edges.len() < 3 || !is_cyclic(edges) {
        return RouteNote::None;
    }
    if opts.lftj.enabled {
        // M4 plugs an operator in here; until then the region stays in SQL
        let _ = reg.lftj();
        return RouteNote::LftjUnavailable;
    }
    RouteNote::CyclicLftjDisabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::Error;
    use tm_ir::builder::IrBuilder;
    use tm_ir::{PathExpr, PathMode};

    fn q(root: Op) -> IrQuery {
        IrQuery::sparql(root)
    }

    struct Fake;
    impl crate::native::NativeOperator for Fake {
        fn kind(&self) -> crate::native::NativeKind {
            crate::native::NativeKind::Path
        }
        fn tvf_name(&self) -> &'static str {
            "tm_path"
        }
        fn register(&self, _h: &mut dyn tm_core::HostRegistry) -> tm_core::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn routes() {
        let b = IrBuilder::sparql();
        let path =
            |s: &str, e: &str| b.path(s, PathExpr::iri("urn:p").star(), e, PathMode::Reachability);
        let empty = OperatorRegistry::new();
        let mut reg = OperatorRegistry::new();
        reg.add(std::sync::Arc::new(Fake));
        // no operator: Unsupported naming path patterns
        match precheck(&q(path("?a", "?b")), &empty) {
            Err(Error::Unsupported { feature }) => assert!(feature.contains("path patterns")),
            other => panic!("{other:?}"),
        }
        // no bound endpoint
        assert!(matches!(
            precheck(&q(path("?a", "?b")), &reg),
            Err(Error::Unsupported { .. })
        ));
        // start bound by a triple pattern; end-bound with a constant
        let j = Op::join(vec![b.triple("?x", "v:p", "?a"), path("?a", "?b")]);
        assert!(precheck(&q(j), &reg).is_ok());
        let bound = VarSet::new();
        let p = PathPattern {
            start: TermOrVar::var("s"),
            end: TermOrVar::iri("urn:x"),
            path: PathExpr::iri("urn:p"),
            mode: PathMode::Reachability,
            max_hops: None,
            bind_path: None,
            view: tm_ir::View::NOW,
            graph: tm_ir::GraphSel::Any,
            time_respecting: None,
            hop_cap: false,
        };
        assert_eq!(orient(&p, &bound).unwrap(), Orientation::Inverted);
        let fwd = PathPattern {
            start: TermOrVar::iri("urn:x"),
            ..p.clone()
        };
        assert_eq!(orient(&fwd, &bound).unwrap(), Orientation::Forward);
        // cyclic BGP stays in SQL
        let e = |a: &str, c: &str| {
            [Var::new(a), Var::new(c)]
                .into_iter()
                .collect::<BTreeSet<_>>()
        };
        let tri = [e("a", "b"), e("b", "c"), e("c", "a")];
        let opts = PlannerOptions::default();
        assert_eq!(route_bgp(&tri, &opts, &reg), RouteNote::CyclicLftjDisabled);
        let mut on = opts;
        on.lftj.enabled = true;
        assert_eq!(route_bgp(&tri, &on, &reg), RouteNote::LftjUnavailable);
        assert_eq!(route_bgp(&tri[..2], &opts, &reg), RouteNote::None);
    }
}
