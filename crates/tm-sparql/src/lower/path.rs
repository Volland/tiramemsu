//! Property paths (`path-lowering`). A path with `*`, `+` or `?` goes to the
//! native path operator with `REACH` semantics; a path of only `/`, `|`, `^` and
//! IRIs uses the SPARQL 1.1 translation to joins and unions of triple patterns, so
//! it needs no bound endpoint. Both carry the graph selection of their block.
//! Negated property sets are `Unsupported`.

use spargebra::algebra::PropertyPathExpression as P;
use spargebra::term::TermPattern;
use tm_core::{Result, Value};
use tm_ir::{
    GraphSel, Op, PathExpr, PathMode, PathPattern, TemporalPath, TermOrVar, TriplePattern, View,
};

use super::Lowerer;
use crate::error::{temporal_path_error, unsupported, NEGATED_PROPERTY_SET, PROPERTY_PATH};

/// Sets the arrival variable of every time-respecting path ending at `end`
/// (counting them in `found`); paths inside expressions are not considered.
fn bind_arrival(op: &mut Op, end: &TermOrVar, var: &tm_ir::Var, found: &mut u32) {
    if let Op::Path(p) = op {
        if let Some(t) = p.time_respecting.as_mut() {
            if &p.end == end && t.arrival.is_none() {
                t.arrival = Some(var.clone());
                *found += 1;
            } else if &p.end == end {
                *found += 1;
            }
        }
    }
    for c in op.children_mut() {
        bind_arrival(c, end, var, found);
    }
}

fn show(t: &TermOrVar) -> String {
    match t {
        TermOrVar::Var(v) => format!("?{}", v.name()),
        TermOrVar::Const(c) => c.to_string(),
        other => format!("{other:?}"),
    }
}

/// True when the path contains `*`, `+` or `?`.
fn recursive(p: &P) -> bool {
    match p {
        P::ZeroOrMore(_) | P::OneOrMore(_) | P::ZeroOrOne(_) => true,
        P::Reverse(x) => recursive(x),
        P::Sequence(a, b) | P::Alternative(a, b) => recursive(a) || recursive(b),
        P::NamedNode(_) | P::NegatedPropertySet(_) => false,
    }
}

/// The IR path expression of `p`.
fn expr(p: &P) -> Result<PathExpr> {
    Ok(match p {
        P::NamedNode(n) => PathExpr::iri(n.as_str()),
        P::Reverse(x) => expr(x)?.inverse(),
        P::Sequence(a, b) => seq_or_alt(expr(a)?, expr(b)?, true),
        P::Alternative(a, b) => seq_or_alt(expr(a)?, expr(b)?, false),
        P::ZeroOrMore(x) => PathExpr::ZeroOrMore(Box::new(expr(x)?)),
        P::OneOrMore(x) => PathExpr::OneOrMore(Box::new(expr(x)?)),
        P::ZeroOrOne(x) => PathExpr::ZeroOrOne(Box::new(expr(x)?)),
        P::NegatedPropertySet(_) => return Err(unsupported(NEGATED_PROPERTY_SET)),
    })
}

/// Flattens nested sequences (alternatives) into one node.
fn seq_or_alt(a: PathExpr, b: PathExpr, seq: bool) -> PathExpr {
    let mut items = Vec::new();
    for x in [a, b] {
        match x {
            PathExpr::Seq(xs) if seq => items.extend(xs),
            PathExpr::Alt(xs) if !seq => items.extend(xs),
            other => items.push(other),
        }
    }
    if seq {
        PathExpr::Seq(items)
    } else {
        PathExpr::Alt(items)
    }
}

impl Lowerer<'_> {
    /// Lowers `subject path object` under `view`, in the active graph selection: a
    /// recursive path filters every hop by it, the translation of a non-recursive
    /// one puts it on each triple pattern.
    // @lat: [[query#Physical Planning#Path Engine#Path Lowering]]
    pub fn path(
        &mut self,
        subject: &TermPattern,
        path: &P,
        object: &TermPattern,
        view: View,
    ) -> Result<Op> {
        let s = self.simple_position(subject)?;
        let o = self.simple_position(object)?;
        // inside a time-respecting scope every path the parser hands over (a plain
        // sequence `a/b` arrives as triple patterns) is one time-respecting region
        if recursive(path) || self.temporal.is_some() {
            if matches!(self.active, GraphSel::Var(_)) {
                self.graph_var_uses += 1;
            }
            let time_respecting = self.temporal.as_mut().map(|t| {
                t.paths += 1;
                TemporalPath {
                    after: t.after.clone(),
                    arrival: None,
                }
            });
            return Ok(Op::Path(PathPattern {
                start: s,
                end: o,
                path: expr(path)?,
                mode: PathMode::Reachability,
                max_hops: None,
                bind_path: None,
                view,
                graph: self.active.clone(),
                time_respecting,
                hop_cap: false,
            }));
        }
        self.expand(s, path, o, view)
    }

    /// The SPARQL 1.1 translation of a non-recursive path: a sequence joins on a
    /// fresh variable, an alternative is a union, an inverse swaps its ends.
    fn expand(&mut self, s: TermOrVar, p: &P, o: TermOrVar, view: View) -> Result<Op> {
        Ok(match p {
            P::NamedNode(n) => Op::Triple(self.select_graph(
                TriplePattern::new(
                    s,
                    TermOrVar::Const(Value::Iri(n.as_str().to_string())),
                    o,
                    view,
                ),
                &[],
            )),
            P::Reverse(x) => self.expand(o, x, s, view)?,
            P::Sequence(a, b) => {
                let mid = TermOrVar::Var(self.vars.fresh("p"));
                let left = self.expand(s, a, mid.clone(), view)?;
                let right = self.expand(mid, b, o, view)?;
                Op::join(vec![left, right])
            }
            P::Alternative(a, b) => {
                let left = self.expand(s.clone(), a, o.clone(), view)?;
                let right = self.expand(s, b, o, view)?;
                Op::union(vec![left, right])
            }
            P::NegatedPropertySet(_) => return Err(unsupported(NEGATED_PROPERTY_SET)),
            P::ZeroOrMore(_) | P::OneOrMore(_) | P::ZeroOrOne(_) => {
                unreachable!("recursive paths are routed to the path operator")
            }
        })
    }

    /// Lowers the group of `SERVICE <urn:tiramemsu:tm:timeRespecting…> { … }`: its
    /// paths become time-respecting from `after`, and each `?end tm:arrival ?t` of
    /// the group binds the arrival of the one time-respecting path ending at `?end`.
    // @lat: [[query#Temporal Path Syntax#SPARQL Temporal Paths]]
    pub fn temporal_scope(
        &mut self,
        after: Option<TermOrVar>,
        inner: &spargebra::algebra::GraphPattern,
        sc: crate::dataset::ViewScope,
    ) -> Result<Op> {
        let outer = self.temporal.replace(super::TemporalScope {
            after,
            ..Default::default()
        });
        let op = self.pattern(inner, sc);
        let scope = std::mem::replace(&mut self.temporal, outer).unwrap_or_default();
        let mut op = op?;
        if scope.paths == 0 {
            return Err(temporal_path_error(
                "the SERVICE <urn:tiramemsu:tm:timeRespecting> group has no property path \
                 (write `*`, `+`, `?`, `^` or `|`; a plain sequence `a/b` is triple patterns)",
            ));
        }
        for (end, var) in scope.arrivals {
            let mut found = 0;
            bind_arrival(&mut op, &end, &var, &mut found);
            match found {
                1 => {}
                0 => {
                    return Err(temporal_path_error(format!(
                        "tm:arrival names {}, which is the end of no time-respecting path \
                         of the group",
                        show(&end)
                    )))
                }
                _ => {
                    return Err(temporal_path_error(format!(
                        "tm:arrival names {}, which ends several time-respecting paths",
                        show(&end)
                    )))
                }
            }
        }
        Ok(op)
    }

    fn simple_position(&mut self, t: &TermPattern) -> Result<TermOrVar> {
        Ok(match t {
            TermPattern::NamedNode(n) => TermOrVar::Const(crate::terms::named_node(n)),
            TermPattern::Literal(l) => TermOrVar::Const(crate::terms::literal(l)?),
            TermPattern::Variable(v) => TermOrVar::Var(tm_ir::Var::new(v.as_str())),
            TermPattern::BlankNode(b) => TermOrVar::Var(self.bnode_var(b.as_str())),
            TermPattern::Triple(_) => return Err(unsupported(PROPERTY_PATH)),
        })
    }
}
