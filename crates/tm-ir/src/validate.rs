//! Structural validation and variable scoping.
//!
//! [`validate`] rejects structurally invalid IR with `InvalidQuery` before
//! anything runs. [`scope`] computes, per operator, the variables it exposes in
//! order of first binding (a left-to-right depth-first walk) and the ones that may
//! be missing; [`output_vars`] gives the result columns.

use tm_core::{Error, Result, Value};

use crate::expr::Expr;
use crate::op::{IrQuery, Op};
use crate::term::TermOrVar;
use crate::var::{Var, VarSet};
use crate::vocab::is_virtual;

/// The variables an operator exposes and which of them may be missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    /// Exposed variables in order of first binding.
    pub vars: Vec<Var>,
    /// Exposed variables that may be missing in some row.
    pub maybe_missing: VarSet,
}

impl Scope {
    fn push(&mut self, v: &Var) {
        if !self.vars.contains(v) {
            self.vars.push(v.clone());
        }
    }

    /// True when `v` is exposed.
    pub fn binds(&self, v: &Var) -> bool {
        self.vars.contains(v)
    }
}

fn term_var(t: &TermOrVar, out: &mut Scope) {
    if let TermOrVar::Var(v) = t {
        out.push(v);
    }
}

/// The scope of an operator.
pub fn scope(op: &Op) -> Scope {
    let mut s = Scope::default();
    match op {
        Op::Triple(t) => {
            term_var(&t.s, &mut s);
            term_var(&t.p, &mut s);
            term_var(&t.o, &mut s);
            if let Some(e) = &t.eid {
                s.push(e);
            }
            if let crate::op::GraphSel::Var(g) = &t.graph {
                s.push(g);
            }
        }
        Op::Path(p) => {
            term_var(&p.start, &mut s);
            term_var(&p.end, &mut s);
            if let Some(b) = &p.bind_path {
                s.push(b);
            }
            if let crate::op::GraphSel::Var(g) = &p.graph {
                s.push(g);
            }
        }
        Op::Values(v) => {
            for (i, var) in v.vars.iter().enumerate() {
                s.push(var);
                if v.rows.iter().any(|r| r.get(i).is_none_or(Option::is_none)) {
                    s.maybe_missing.insert(var.clone());
                }
            }
        }
        Op::Unnest(u) => {
            s = scope(&u.input);
            s.push(&u.var);
        }
        Op::Join(j) => {
            let scopes: Vec<Scope> = j.inputs.iter().map(scope).collect();
            for sc in &scopes {
                for v in &sc.vars {
                    s.push(v);
                }
            }
            for v in &s.vars {
                let binding: Vec<&Scope> = scopes.iter().filter(|sc| sc.binds(v)).collect();
                if binding.iter().all(|sc| sc.maybe_missing.contains(v)) {
                    s.maybe_missing.insert(v.clone());
                }
            }
        }
        Op::LeftJoin(l) => {
            let a = scope(&l.left);
            let b = scope(&l.right);
            for v in a.vars.iter().chain(&b.vars) {
                s.push(v);
            }
            s.maybe_missing = a.maybe_missing.clone();
            for v in &b.vars {
                if !a.binds(v) {
                    s.maybe_missing.insert(v.clone());
                }
            }
        }
        Op::Filter(f) => s = scope(&f.input),
        Op::Union(u) => {
            let scopes: Vec<Scope> = u.inputs.iter().map(scope).collect();
            for sc in &scopes {
                for v in &sc.vars {
                    s.push(v);
                }
            }
            for v in &s.vars {
                if scopes
                    .iter()
                    .any(|sc| !sc.binds(v) || sc.maybe_missing.contains(v))
                {
                    s.maybe_missing.insert(v.clone());
                }
            }
        }
        Op::Extend(e) => {
            s = scope(&e.input);
            s.push(&e.var);
            let certain = match &e.expr {
                Expr::Const(_) | Expr::Param(_) => true,
                Expr::Var(v) => {
                    let inner = scope(&e.input);
                    inner.binds(v) && !inner.maybe_missing.contains(v)
                }
                _ => false,
            };
            if !certain {
                s.maybe_missing.insert(e.var.clone());
            }
        }
        Op::Aggregate(a) => {
            let inner = scope(&a.input);
            for g in &a.group {
                s.push(g);
                if !inner.binds(g) || inner.maybe_missing.contains(g) {
                    s.maybe_missing.insert(g.clone());
                }
            }
            for agg in &a.aggs {
                s.push(&agg.var);
                if agg.func != crate::agg::AggFunc::Count
                    && agg.func != crate::agg::AggFunc::Collect
                {
                    s.maybe_missing.insert(agg.var.clone());
                }
            }
        }
        Op::Project(p) => {
            let inner = scope(&p.input);
            for v in &p.vars {
                s.push(v);
                if !inner.binds(v) || inner.maybe_missing.contains(v) {
                    s.maybe_missing.insert(v.clone());
                }
            }
        }
        Op::OrderLimit(o) => s = scope(&o.input),
        Op::RowNumber(r) => {
            s = scope(&r.input);
            s.push(&r.var);
        }
    }
    s
}

/// The result columns: the root `Project`'s variables in declared order, otherwise
/// every exposed variable in order of first binding.
pub fn output_vars(op: &Op) -> Vec<Var> {
    scope(op).vars
}

fn invalid(msg: String) -> Error {
    Error::InvalidQuery { msg }
}

/// Checks a skip or limit position: a non-negative integer constant, or a parameter
/// (checked after binding).
pub fn check_count(what: &str, t: &TermOrVar) -> Result<()> {
    match t {
        TermOrVar::Const(Value::Int(n)) if *n >= 0 => Ok(()),
        TermOrVar::Param(_) => Ok(()),
        TermOrVar::Const(Value::Int(n)) => Err(invalid(format!("negative {what} {n}"))),
        other => Err(invalid(format!(
            "{what} must be a non-negative integer, got {other:?}"
        ))),
    }
}

fn check_expr(e: &Expr) -> Result<()> {
    match e {
        Expr::Exists(op, _) => check_op(op),
        Expr::Var(_) | Expr::Const(_) | Expr::Param(_) | Expr::Bound(_) => Ok(()),
        Expr::Cmp(_, a, b) | Expr::SameTerm(a, b) | Expr::Arith(_, a, b) => {
            check_expr(a)?;
            check_expr(b)
        }
        Expr::And(xs) | Expr::Or(xs) | Expr::Coalesce(xs) | Expr::List(xs) | Expr::Func(_, xs) => {
            xs.iter().try_for_each(check_expr)
        }
        Expr::Not(a) | Expr::Neg(a) => check_expr(a),
        Expr::In(a, xs, _) => {
            check_expr(a)?;
            xs.iter().try_for_each(check_expr)
        }
        Expr::If(a, b, c) => {
            check_expr(a)?;
            check_expr(b)?;
            check_expr(c)
        }
        Expr::Lookup(l) => {
            if let TermOrVar::Var(v) = &l.pred {
                return Err(invalid(format!("lookup predicate {v} must be a constant")));
            }
            check_expr(&l.subject)
        }
    }
}

/// A graph set names at least one graph, by constants and parameters only.
fn check_graph(g: &crate::op::GraphSel) -> Result<()> {
    match g {
        crate::op::GraphSel::Set(gs) if gs.is_empty() => Err(invalid(
            "a graph set must name at least one graph".to_string(),
        )),
        crate::op::GraphSel::Set(gs)
            if gs
                .iter()
                .any(|g| !matches!(g, TermOrVar::Const(_) | TermOrVar::Param(_))) =>
        {
            Err(invalid(
                "a graph set holds constants and parameters only".to_string(),
            ))
        }
        _ => Ok(()),
    }
}

fn check_op(op: &Op) -> Result<()> {
    match op {
        Op::Triple(t) => {
            check_graph(&t.graph)?;
            if let (TermOrVar::Const(Value::Iri(p)), true) = (&t.p, !t.graph.is_any()) {
                if is_virtual(p) {
                    return Err(invalid(format!(
                        "virtual predicate <{p}> cannot be selected by graph"
                    )));
                }
            }
            if let (TermOrVar::Const(Value::Iri(p)), Some(e)) = (&t.p, &t.eid) {
                if is_virtual(p) {
                    return Err(invalid(format!(
                        "virtual predicate <{p}> cannot bind an eid ({e})"
                    )));
                }
            }
        }
        Op::Path(p) => check_graph(&p.graph)?,
        Op::Values(v) => {
            for (i, r) in v.rows.iter().enumerate() {
                if r.len() != v.vars.len() {
                    return Err(invalid(format!(
                        "Values row {i} has {} cells for {} variables",
                        r.len(),
                        v.vars.len()
                    )));
                }
                if let Some(Some(TermOrVar::Var(x))) =
                    r.iter().find(|c| matches!(c, Some(TermOrVar::Var(_))))
                {
                    return Err(invalid(format!("Values cell cannot be a variable ({x})")));
                }
            }
            let mut seen = VarSet::new();
            for x in &v.vars {
                if !seen.insert(x.clone()) {
                    return Err(invalid(format!("Values repeats variable {x}")));
                }
            }
        }
        Op::Unnest(u) => {
            if scope(&u.input).binds(&u.var) {
                return Err(invalid(format!("Unnest rebinds {}", u.var)));
            }
            check_expr(&u.list)?;
        }
        Op::Join(_) | Op::Union(_) => {}
        Op::LeftJoin(l) => {
            if let Some(c) = &l.cond {
                check_expr(c)?;
            }
        }
        Op::Filter(f) => check_expr(&f.cond)?,
        Op::Extend(e) => {
            if scope(&e.input).binds(&e.var) {
                return Err(invalid(format!(
                    "Extend binds {} which its input already binds",
                    e.var
                )));
            }
            check_expr(&e.expr)?;
        }
        Op::Aggregate(a) => {
            let mut seen: VarSet = a.group.iter().cloned().collect();
            for agg in &a.aggs {
                if !seen.insert(agg.var.clone()) {
                    return Err(invalid(format!(
                        "aggregate output {} collides with a grouping or aggregate variable",
                        agg.var
                    )));
                }
                if agg.arg.is_none() && agg.func != crate::agg::AggFunc::Count {
                    return Err(invalid(format!("{} needs an argument", agg.func.name())));
                }
                if let Some(x) = &agg.arg {
                    check_expr(x)?;
                }
            }
        }
        Op::Project(_) => {}
        Op::OrderLimit(o) => {
            if let Some(s) = &o.skip {
                check_count("skip", s)?;
            }
            if let Some(l) = &o.limit {
                check_count("limit", l)?;
            }
            for k in &o.keys {
                check_expr(&k.expr)?;
            }
        }
        Op::RowNumber(r) => {
            if scope(&r.input).binds(&r.var) {
                return Err(invalid(format!("RowNumber rebinds {}", r.var)));
            }
            for k in &r.order {
                check_expr(&k.expr)?;
            }
        }
    }
    op.children().into_iter().try_for_each(check_op)
}

/// Rejects a structurally invalid IR with `InvalidQuery`: an Extend (or Unnest,
/// RowNumber) that rebinds a variable its input binds, an aggregate output that
/// collides with a grouping variable, a Values row whose width differs from its
/// variable list, a negative skip or limit, and a virtual-predicate pattern that
/// binds an eid.
///
/// Run this on every tree a front end produces; the executor runs it again
/// before planning. It needs no database.
///
/// # Errors
///
/// Returns `Error::InvalidQuery` for the first problem found.
pub fn validate(q: &IrQuery) -> Result<()> {
    check_op(&q.root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agg::Agg;
    use crate::builder::IrBuilder;
    use crate::op::{TriplePattern, Values};
    use crate::view::View;

    fn b() -> IrBuilder {
        IrBuilder::sparql()
    }

    fn is_invalid(op: Op) -> bool {
        matches!(
            validate(&IrQuery::sparql(op)),
            Err(Error::InvalidQuery { .. })
        )
    }

    // query-ir "Extend rebinds a bound variable"
    #[test]
    fn extend_rebinding() {
        let op = b()
            .triple("?a", "v:p", "?b")
            .extend("a", Expr::val(Value::Int(1)));
        assert!(is_invalid(op));
        let ok = b()
            .triple("?a", "v:p", "?b")
            .extend("c", Expr::val(Value::Int(1)));
        assert!(validate(&IrQuery::sparql(ok)).is_ok());
    }

    #[test]
    fn aggregate_collision() {
        let op = b()
            .triple("?a", "v:p", "?b")
            .aggregate(&["a"], vec![Agg::count_star("a")]);
        assert!(is_invalid(op));
    }

    // query-ir "Ragged Values"
    #[test]
    fn ragged_values() {
        let c = |i: i64| Some(TermOrVar::Const(Value::Int(i)));
        let op = Op::Values(Values {
            vars: vec!["x".into(), "y".into()],
            rows: vec![vec![c(1), c(2)], vec![c(1), c(2), c(3)]],
        });
        assert!(is_invalid(op));
    }

    #[test]
    fn negative_skip_and_limit() {
        let op = b()
            .triple("?a", "v:p", "?b")
            .order_limit(vec![], Some(-1), None);
        assert!(is_invalid(op));
        let op = b()
            .triple("?a", "v:p", "?b")
            .order_limit(vec![], None, Some(-5));
        assert!(is_invalid(op));
    }

    // @lat: [[tests#Named Graphs#Graph Selector Text And Validation]]
    #[test]
    fn graph_selector_text_and_validation() {
        use crate::op::GraphSel;
        let any = TriplePattern::new("?s", "v:p", "?o", View::now());
        let set = |gs: Vec<TermOrVar>| Op::Triple(any.clone().in_graph(GraphSel::Set(gs)));
        assert_eq!(
            crate::display::sexpr_line(&Op::Triple(any.clone())),
            "(triple ?s <v:p> ?o :view now)",
        );
        let text = crate::display::sexpr_line(&set(vec![TermOrVar::iri("urn:g1"), "$g".into()]));
        assert!(text.ends_with(":graph (<urn:g1> $g))"), "{text}");
        let var = Op::Triple(any.clone().in_graph(GraphSel::Var("g".into())));
        assert!(crate::display::sexpr_line(&var).ends_with(":graph ?g)"));
        assert!(scope(&var).binds(&"g".into()));
        assert!(is_invalid(set(vec![])));
        assert!(is_invalid(set(vec!["?x".into()])));
        let virt = TriplePattern::new("?r", crate::vocab::TM_TX_ADDED, "?t", View::now())
            .in_graph(GraphSel::Set(vec![TermOrVar::iri("urn:g1")]));
        assert!(is_invalid(Op::Triple(virt)));
        assert!(!is_invalid(set(vec![TermOrVar::iri("urn:g1")])));
    }

    // @lat: [[tests#Named Graphs#Path Graph Selector Text And Validation]]
    #[test]
    fn path_graph_selector_text_and_validation() {
        use crate::op::GraphSel;
        let path = |g: GraphSel| match b().path(
            "v:a",
            crate::path::PathExpr::iri("v:knows").plus(),
            "?x",
            crate::path::PathMode::Reachability,
        ) {
            Op::Path(p) => Op::Path(p.in_graph(g)),
            _ => unreachable!("a path"),
        };
        let any = crate::display::sexpr_line(&path(GraphSel::Any));
        assert!(!any.contains(":graph"), "Any prints nothing: {any}");
        let set = path(GraphSel::Set(vec![TermOrVar::iri("urn:g1"), "$g".into()]));
        let text = crate::display::sexpr_line(&set);
        assert!(text.ends_with(":graph (<urn:g1> $g))"), "{text}");
        assert!(!is_invalid(set));
        let var = path(GraphSel::Var("g".into()));
        assert!(crate::display::sexpr_line(&var).ends_with(":graph ?g)"));
        assert!(scope(&var).binds(&"g".into()));
        assert!(!is_invalid(var));
        assert!(is_invalid(path(GraphSel::Set(vec![]))));
        assert!(is_invalid(path(GraphSel::Set(vec!["?x".into()]))));
    }

    // virtual-predicates "Eid on a virtual pattern" (validation half)
    #[test]
    fn virtual_pattern_with_eid() {
        let op = Op::Triple(
            TriplePattern::new("?r", crate::vocab::TM_TX_ADDED, "?t", View::now()).with_eid("?x"),
        );
        assert!(is_invalid(op));
    }

    #[test]
    fn nested_exists_is_validated() {
        let bad = b()
            .triple("?a", "v:p", "?b")
            .extend("a", Expr::val(Value::Int(1)));
        let op = b().triple("?a", "v:p", "?b").filter(Expr::exists(bad));
        assert!(is_invalid(op));
    }

    // query-ir "Project fixes column order"
    #[test]
    fn project_order() {
        let op = b().bgp(&[("?a", "v:p", "?n")]).project(&["n", "a"]);
        assert_eq!(output_vars(&op), vec![Var::new("n"), Var::new("a")]);
    }

    // query-ir "Implicit column order"
    #[test]
    fn implicit_first_binding_order() {
        let op = b().bgp(&[("?a", "v:p", "?b"), ("?b", "v:q", "?c")]);
        assert_eq!(
            output_vars(&op),
            vec![Var::new("a"), Var::new("b"), Var::new("c")]
        );
        let op = Op::left_join(
            b().triple("?p", "v:name", "?n"),
            Op::Triple(b().t("?p", "v:email", "?e").with_eid("?r")),
            None,
        )
        .extend("x", Expr::val(Value::Int(1)));
        let s = scope(&op);
        assert_eq!(
            s.vars,
            vec![
                Var::new("p"),
                Var::new("n"),
                Var::new("e"),
                Var::new("r"),
                Var::new("x")
            ]
        );
        assert!(s.maybe_missing.contains(&Var::new("e")));
        assert!(!s.maybe_missing.contains(&Var::new("n")));
        assert!(!s.maybe_missing.contains(&Var::new("x")));
    }

    #[test]
    fn union_and_join_missing_sets() {
        let u = Op::union(vec![
            b().triple("?x", "v:p", "?y"),
            b().triple("?x", "v:q", "?z"),
        ]);
        let s = scope(&u);
        assert_eq!(s.vars, vec![Var::new("x"), Var::new("y"), Var::new("z")]);
        assert!(s.maybe_missing.contains(&Var::new("y")));
        assert!(!s.maybe_missing.contains(&Var::new("x")));
        // a join with a certain binding makes the variable certain
        let j = Op::join(vec![u, b().triple("?y", "v:r", "?w")]);
        assert!(!scope(&j).maybe_missing.contains(&Var::new("y")));
    }
}
