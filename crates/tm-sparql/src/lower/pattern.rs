//! Graph patterns → IR operators (design D5): joins, optionals, filters, unions,
//! `SERVICE` time scopes, `MINUS`, `VALUES` and the solution modifiers.

use spargebra::algebra::{GraphPattern, OrderExpression};
use spargebra::term::{GroundTerm, NamedNodePattern, TriplePattern as SpTriple};
use tm_core::{Result, Value};
use tm_ir::validate::scope;
use tm_ir::{Expr, GraphSel, Join, Key, Op, TermOrVar, Values, Var};

use super::bgp::{empty_op, is_empty_op};
use super::vars::is_internal;
use super::Lowerer;
use crate::dataset::{graph_name, is_tm_iri, ViewScope};
use crate::error::{
    time_iri_in_graph, unsupported, GRAPH_SUBQUERY, GRAPH_WITHOUT_PATTERN, ORDER_BY_DISTINCT,
    SERVICE,
};
use crate::terms;

/// One operand of a flattened join.
enum Operand<'p> {
    Bgp(Vec<&'p SpTriple>),
    Other(&'p GraphPattern),
}

fn flatten_join<'p>(gp: &'p GraphPattern, out: &mut Vec<Operand<'p>>) {
    match gp {
        GraphPattern::Join { left, right } => {
            flatten_join(left, out);
            flatten_join(right, out);
        }
        GraphPattern::Bgp { patterns } => match out.last_mut() {
            Some(Operand::Bgp(prev)) => prev.extend(patterns.iter()),
            _ => out.push(Operand::Bgp(patterns.iter().collect())),
        },
        other => out.push(Operand::Other(other)),
    }
}

fn flatten_union<'p>(gp: &'p GraphPattern, out: &mut Vec<&'p GraphPattern>) {
    match gp {
        GraphPattern::Union { left, right } => {
            flatten_union(left, out);
            flatten_union(right, out);
        }
        other => out.push(other),
    }
}

/// Splices nested plain joins and drops unit operands.
fn join_of(ops: Vec<Op>) -> Op {
    let mut flat = Vec::new();
    for op in ops {
        match op {
            Op::Join(Join { inputs, null_safe }) if null_safe.is_empty() => flat.extend(inputs),
            other => flat.push(other),
        }
    }
    if flat.iter().any(is_empty_op) {
        return empty_op();
    }
    if flat.len() == 1 {
        return flat.pop().expect("one");
    }
    Op::join(flat)
}

impl Lowerer<'_> {
    /// Lowers `gp` with the time scope `sc`.
    pub fn pattern(&mut self, gp: &GraphPattern, sc: ViewScope) -> Result<Op> {
        let view = sc.resolve(self.env.base_view);
        match gp {
            GraphPattern::Bgp { patterns } => {
                let refs: Vec<&SpTriple> = patterns.iter().collect();
                if refs.is_empty() {
                    return Ok(Op::unit());
                }
                self.bgp(&refs, view)
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            } => self.path(subject, path, object, view),
            GraphPattern::Join { .. } => {
                let mut operands = Vec::new();
                flatten_join(gp, &mut operands);
                let mut ops = Vec::new();
                for o in operands {
                    ops.push(match o {
                        Operand::Bgp(refs) => {
                            if refs.is_empty() {
                                continue;
                            }
                            self.bgp(&refs, view)?
                        }
                        Operand::Other(p) => self.pattern(p, sc)?,
                    });
                }
                Ok(join_of(ops))
            }
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                let l = self.pattern(left, sc)?;
                let r = self.pattern(right, sc)?;
                let cond = match expression {
                    Some(e) => Some(self.expr(e, sc)?),
                    None => None,
                };
                Ok(Op::left_join(l, r, cond))
            }
            GraphPattern::Filter { expr, inner } => {
                let op = self.pattern(inner, sc)?;
                let cond = self.expr(expr, sc)?;
                Ok(op.filter(cond))
            }
            GraphPattern::Union { .. } => {
                let mut branches = Vec::new();
                flatten_union(gp, &mut branches);
                let ops: Result<Vec<Op>> =
                    branches.into_iter().map(|b| self.pattern(b, sc)).collect();
                Ok(Op::union(ops?))
            }
            GraphPattern::Extend {
                inner,
                variable,
                expression,
            } => {
                let op = self.pattern(inner, sc)?;
                let e = self.expr(expression, sc)?;
                Ok(op.extend(self.var(variable.as_str()).name(), e))
            }
            GraphPattern::Minus { left, right } => {
                let l = self.pattern(left, sc)?;
                let r = self.pattern(right, sc)?;
                let ls = scope(&l);
                let rs = scope(&r);
                // the graph variable of an enclosing `GRAPH ?g` block correlates the two
                // sides but is not a variable of the solutions MINUS compares
                let shared: Vec<Var> = ls
                    .vars
                    .iter()
                    .filter(|v| rs.binds(v) && !super::vars::is_graph_var(v))
                    .cloned()
                    .collect();
                if shared.is_empty() {
                    // no shared variable: MINUS removes nothing
                    return Ok(l);
                }
                // A right solution removes a left one only when it is compatible AND
                // the two share a variable bound on both sides. The correlation of
                // `NOT EXISTS` gives compatibility; the domain condition is added on
                // the right side: some shared variable must be bound there. (A
                // variable that may be unbound on the left is not counted, unless
                // every shared variable may be.)
                let certain: Vec<&Var> = shared
                    .iter()
                    .filter(|v| !ls.maybe_missing.contains(*v))
                    .collect();
                let overlap: Vec<&Var> = if certain.is_empty() {
                    shared.iter().collect()
                } else {
                    certain
                };
                let always_bound = overlap.iter().any(|v| !rs.maybe_missing.contains(*v));
                let r = if always_bound {
                    r
                } else {
                    let any_bound: Vec<Expr> =
                        overlap.iter().map(|v| Expr::Bound((*v).clone())).collect();
                    r.filter(Expr::Or(any_bound))
                };
                Ok(l.filter(Expr::not_exists(r)))
            }
            GraphPattern::Values {
                variables,
                bindings,
            } => self.values(variables, bindings),
            GraphPattern::Service { name, inner, .. } => match name {
                NamedNodePattern::NamedNode(n)
                    if crate::dataset::parse_time_respecting_iri(n.as_str())?.is_some() =>
                {
                    let after = crate::dataset::parse_time_respecting_iri(n.as_str())?.flatten();
                    self.temporal_scope(after, inner, sc)
                }
                NamedNodePattern::NamedNode(n) => match sc.enter_service(n)? {
                    Some(inner_scope) => self.pattern(inner, inner_scope),
                    None => Err(unsupported(SERVICE)),
                },
                NamedNodePattern::Variable(_) => Err(unsupported(SERVICE)),
            },
            GraphPattern::Graph { name, inner } => match name {
                NamedNodePattern::NamedNode(n) if is_tm_iri(n.as_str()) => {
                    Err(time_iri_in_graph(n.as_str()))
                }
                NamedNodePattern::NamedNode(n) => self.graph_block(n.as_str(), inner, sc),
                NamedNodePattern::Variable(v) => self.graph_var_block(v.as_str(), inner, sc),
            },
            GraphPattern::Group {
                inner,
                variables,
                aggregates,
            } => self.group(inner, variables, aggregates, sc),
            GraphPattern::OrderBy { .. }
            | GraphPattern::Project { .. }
            | GraphPattern::Distinct { .. }
            | GraphPattern::Reduced { .. }
            | GraphPattern::Slice { .. } => self.modifiers(gp, sc),
        }
    }

    /// `GRAPH <g> { … }`: the block's statements must be members of `g`. A graph that
    /// a `FROM NAMED` list leaves out matches nothing (the block keeps its variables).
    fn graph_block(&mut self, iri: &str, inner: &GraphPattern, sc: ViewScope) -> Result<Op> {
        let g = graph_name(iri)?;
        let hidden = self
            .dataset
            .named
            .as_ref()
            .is_some_and(|list| !list.contains(&g));
        let outer = std::mem::replace(&mut self.active, GraphSel::Set(vec![TermOrVar::Const(g)]));
        let op = self.pattern(inner, sc);
        self.active = outer;
        let op = op?;
        Ok(if hidden {
            op.filter(Expr::Const(Value::Bool(false)))
        } else {
            op
        })
    }

    /// `GRAPH ?g { … }`: one solution per membership, `?g` bound to the graph. The
    /// block's patterns share an internal graph variable, so a `MINUS` or `OPTIONAL`
    /// inside compares solutions of one graph and never sees `?g` as a shared
    /// variable. A `FROM NAMED` list restricts the graphs `?g` ranges over.
    fn graph_var_block(&mut self, name: &str, inner: &GraphPattern, sc: ViewScope) -> Result<Op> {
        let user = self.var(name);
        let internal = self.vars.fresh("g");
        let outer = std::mem::replace(&mut self.active, GraphSel::Var(internal.clone()));
        let used_before = self.graph_var_uses;
        let op = self.pattern(inner, sc);
        self.active = outer;
        let op = op?;
        if self.graph_var_uses == used_before {
            return Err(unsupported(GRAPH_WITHOUT_PATTERN));
        }
        if !scope(&op).binds(&internal) {
            // a subquery hides the graph variable of its patterns
            return Err(unsupported(GRAPH_SUBQUERY));
        }
        let op = if scope(&op).binds(&user) {
            // `GRAPH ?g { ?g :p ?o }`: the graph is also a node of the block
            op.filter(Expr::SameTerm(
                Box::new(Expr::Var(user.clone())),
                Box::new(Expr::Var(internal)),
            ))
        } else {
            op.extend(user.name(), Expr::Var(internal))
        };
        Ok(match &self.dataset.named {
            Some(list) => op.filter(Expr::In(
                Box::new(Expr::Var(user)),
                list.iter().cloned().map(Expr::Const).collect(),
                false,
            )),
            None => op,
        })
    }

    fn values(
        &mut self,
        variables: &[spargebra::term::Variable],
        bindings: &[Vec<Option<GroundTerm>>],
    ) -> Result<Op> {
        let vars: Vec<Var> = variables.iter().map(|v| Var::new(v.as_str())).collect();
        let mut rows = Vec::new();
        for b in bindings {
            let mut row = Vec::new();
            for cell in b {
                row.push(match cell {
                    None => None,
                    Some(GroundTerm::NamedNode(n)) => Some(TermOrVar::Const(terms::named_node(n))),
                    Some(GroundTerm::Literal(l)) => Some(TermOrVar::Const(terms::literal(l)?)),
                    Some(GroundTerm::Triple(_)) => {
                        return Err(unsupported("triple term in VALUES"))
                    }
                });
            }
            rows.push(row);
        }
        Ok(Op::Values(Values { vars, rows }))
    }

    fn keys(&mut self, exprs: &[OrderExpression], sc: ViewScope) -> Result<Vec<Key>> {
        exprs
            .iter()
            .map(|o| {
                Ok(match o {
                    OrderExpression::Asc(e) => Key::asc(self.expr(e, sc)?),
                    OrderExpression::Desc(e) => Key::desc(self.expr(e, sc)?),
                })
            })
            .collect()
    }

    /// `Slice(Distinct?(Project?(OrderBy?(X))))` as one unit (design D5a).
    ///
    /// Without `DISTINCT` the shape is `Project(OrderLimit(X))`, so sort keys may
    /// use any variable of `X`. With `DISTINCT` the slice must follow the
    /// distinct projection, so it is `OrderLimit(Project{distinct}(X))` and the
    /// keys may only use projected variables.
    fn modifiers(&mut self, gp: &GraphPattern, sc: ViewScope) -> Result<Op> {
        let mut cur = gp;
        let (mut skip, mut limit) = (None, None);
        let mut distinct = false;
        let mut project: Option<&Vec<spargebra::term::Variable>> = None;
        let mut order: Option<&Vec<OrderExpression>> = None;
        if let GraphPattern::Slice {
            inner,
            start,
            length,
        } = cur
        {
            skip = (*start > 0).then_some(*start as i64);
            limit = length.map(|l| l as i64);
            cur = inner;
        }
        match cur {
            GraphPattern::Distinct { inner } => {
                distinct = true;
                cur = inner;
            }
            GraphPattern::Reduced { inner } => cur = inner,
            _ => {}
        }
        if let GraphPattern::Project { inner, variables } = cur {
            project = Some(variables);
            cur = inner;
        }
        if let GraphPattern::OrderBy { inner, expression } = cur {
            order = Some(expression);
            cur = inner;
        }
        let base = self.pattern(cur, sc)?;
        let keys = match order {
            Some(o) => self.keys(o, sc)?,
            None => Vec::new(),
        };
        let sliced = !keys.is_empty() || skip.is_some() || limit.is_some();
        let vars: Option<Vec<Var>> =
            project.map(|p| p.iter().map(|v| Var::new(v.as_str())).collect());
        if distinct {
            let Some(vars) = vars else {
                return Err(unsupported("DISTINCT without projection"));
            };
            for k in &keys {
                let mut used = Vec::new();
                k.expr.vars(&mut used);
                if used.iter().any(|u| !vars.contains(u) && !is_internal(u)) {
                    return Err(unsupported(ORDER_BY_DISTINCT));
                }
            }
            let op = Op::Project(tm_ir::Project {
                input: Box::new(base),
                vars,
                distinct: true,
            });
            return Ok(if sliced {
                Op::OrderLimit(tm_ir::OrderLimit {
                    input: Box::new(op),
                    keys,
                    skip: skip.map(|n| TermOrVar::Const(Value::Int(n))),
                    limit: limit.map(|n| TermOrVar::Const(Value::Int(n))),
                })
            } else {
                op
            });
        }
        let mut op = base;
        if sliced {
            op = op.order_limit(keys, skip, limit);
        }
        Ok(match vars {
            Some(vars) => Op::Project(tm_ir::Project {
                input: Box::new(op),
                vars,
                distinct: false,
            }),
            None => op,
        })
    }
}
