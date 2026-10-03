//! Basic graph patterns: triple terms, reifiers and annotations become eid-bound
//! triple patterns (design D6). The `rdf:reifies` desugaring assumptions of
//! spargebra live here and nowhere else.

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern as SpTriple};
use tm_core::Error;
use tm_core::Value;
use tm_ir::{Expr, Op, TermOrVar, TextPattern, TriplePattern, Values, Var, View};

use super::Lowerer;
use crate::error::{unsupported, REIFIES_WITHOUT_TRIPLE, VARIABLE_PREDICATE_TRIPLE};
use crate::terms;
use tm_core::Result;

const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// The zero-row relation (a reifier constant that is not a statement).
pub fn empty_op() -> Op {
    Op::Values(Values {
        vars: Vec::new(),
        rows: Vec::new(),
    })
}

/// True for the zero-row relation built by [`empty_op`].
pub fn is_empty_op(op: &Op) -> bool {
    matches!(op, Op::Values(v) if v.vars.is_empty() && v.rows.is_empty())
}

/// What a reifier position denotes.
enum Reifier {
    /// Binds this eid variable.
    Var(Var),
    /// A fixed statement: bind a fresh variable and require equality.
    Stmt(Value),
    /// Not a statement: the pattern matches nothing.
    Nothing,
}

#[derive(Default)]
struct Items {
    triples: Vec<TriplePattern>,
    filters: Vec<Expr>,
    empty: bool,
    /// Variables that name a reifier: triples about them are annotations, read from
    /// the default view whatever `GRAPH` block they sit in.
    reifiers: Vec<Var>,
}

impl Lowerer<'_> {
    /// Lowers one basic graph pattern read under `view`.
    pub fn bgp(&mut self, patterns: &[&SpTriple], view: View) -> Result<Op> {
        let mut items = Items::default();
        for p in patterns {
            self.one_pattern(p, view, &mut items)?;
        }
        if items.empty {
            return Ok(empty_op());
        }
        let mut triples = std::mem::take(&mut items.triples);
        let texts = self.text_patterns(&mut triples, view)?;
        eliminate_redundant(&mut triples);
        let reifiers = std::mem::take(&mut items.reifiers);
        if triples.is_empty() && texts.is_empty() && items.filters.is_empty() {
            // only tm:arrival patterns
            return Ok(Op::unit());
        }
        let ops: Vec<Op> = triples
            .into_iter()
            .map(|t| Op::Triple(self.select_graph(t, &reifiers)))
            .chain(texts)
            .collect();
        let mut op = if ops.len() == 1 {
            ops.into_iter().next().expect("one")
        } else {
            Op::join(ops)
        };
        for f in items.filters {
            op = op.filter(f);
        }
        Ok(op)
    }

    /// Applies the active graph selection to a statement pattern. Annotation
    /// triples (subject is a reifier of this pattern) and virtual predicates are not
    /// statements of the graph: they read the default view.
    pub(super) fn select_graph(&mut self, t: TriplePattern, reifiers: &[Var]) -> TriplePattern {
        let annotation =
            t.eid.is_none() && matches!(&t.s, TermOrVar::Var(v) if reifiers.contains(v));
        let virtual_pred =
            matches!(&t.p, TermOrVar::Const(Value::Iri(p)) if tm_ir::vocab::is_virtual(p));
        if annotation || virtual_pred {
            t
        } else {
            if matches!(self.active, tm_ir::GraphSel::Var(_)) {
                self.graph_var_uses += 1;
            }
            t.in_graph(self.active.clone())
        }
    }

    fn one_pattern(&mut self, p: &SpTriple, view: View, items: &mut Items) -> Result<()> {
        let is_reifies =
            matches!(&p.predicate, NamedNodePattern::NamedNode(n) if n.as_str() == RDF_REIFIES);
        if is_reifies {
            let TermPattern::Triple(t) = &p.object else {
                return Err(unsupported(REIFIES_WITHOUT_TRIPLE));
            };
            let target = self.reifier(&p.subject);
            if let Reifier::Var(v) = &target {
                items.reifiers.push(v.clone());
            }
            return self.triple_term(t, target, view, items);
        }
        let s = self.position(&p.subject, view, items)?;
        let pred = match &p.predicate {
            NamedNodePattern::NamedNode(n) => TermOrVar::Const(Value::Iri(n.as_str().to_string())),
            NamedNodePattern::Variable(v) => TermOrVar::Var(Var::new(v.as_str())),
        };
        if matches!(&pred, TermOrVar::Const(Value::Iri(i)) if i == tm_ir::vocab::TM_ARRIVAL) {
            return self.arrival_pattern(s, &p.object);
        }
        if matches!(p.object, TermPattern::Triple(_)) && matches!(pred, TermOrVar::Var(_)) {
            return Err(unsupported(VARIABLE_PREDICATE_TRIPLE));
        }
        let o = self.position(&p.object, view, items)?;
        items.triples.push(TriplePattern::new(s, pred, o, view));
        Ok(())
    }

    /// `?end tm:arrival ?t`: recorded for the enclosing time-respecting scope,
    /// which binds `?t` from the path ending at `?end`; no triple pattern.
    fn arrival_pattern(&mut self, end: TermOrVar, object: &TermPattern) -> Result<()> {
        let TermPattern::Variable(v) = object else {
            return Err(crate::error::temporal_path_error(
                "the object of tm:arrival must be a variable",
            ));
        };
        let Some(scope) = self.temporal.as_mut() else {
            return Err(crate::error::temporal_path_error(
                "tm:arrival outside a SERVICE <urn:tiramemsu:tm:timeRespecting> group",
            ));
        };
        let var = Var::new(v.as_str());
        if scope.arrivals.iter().any(|(_, w)| w == &var) {
            return Err(crate::error::temporal_path_error(format!(
                "?{} is bound by two tm:arrival patterns",
                v.as_str()
            )));
        }
        scope.arrivals.push((end, var));
        Ok(())
    }

    /// A pattern position. A triple term becomes a fresh eid variable and the
    /// pattern that binds it.
    fn position(&mut self, t: &TermPattern, view: View, items: &mut Items) -> Result<TermOrVar> {
        Ok(match t {
            TermPattern::NamedNode(n) => TermOrVar::Const(terms::named_node(n)),
            TermPattern::Literal(l) => TermOrVar::Const(terms::literal(l)?),
            TermPattern::Variable(v) => TermOrVar::Var(Var::new(v.as_str())),
            TermPattern::BlankNode(b) => TermOrVar::Var(self.bnode_var(b.as_str())),
            TermPattern::Triple(tp) => {
                let e = self.vars.fresh("e");
                self.triple_term(tp, Reifier::Var(e.clone()), view, items)?;
                TermOrVar::Var(e)
            }
        })
    }

    fn reifier(&mut self, t: &TermPattern) -> Reifier {
        match t {
            TermPattern::Variable(v) => Reifier::Var(Var::new(v.as_str())),
            TermPattern::BlankNode(b) => Reifier::Var(self.bnode_var(b.as_str())),
            TermPattern::NamedNode(n) => match terms::named_node(n) {
                v @ Value::Stmt(_) => Reifier::Stmt(v),
                _ => Reifier::Nothing,
            },
            _ => Reifier::Nothing,
        }
    }

    /// Emits the pattern of the triple term `tp` bound to `target`.
    fn triple_term(
        &mut self,
        tp: &SpTriple,
        target: Reifier,
        view: View,
        items: &mut Items,
    ) -> Result<()> {
        let eid = match target {
            Reifier::Var(v) => v,
            Reifier::Stmt(value) => {
                let e = self.vars.fresh("e");
                items.filters.push(Expr::SameTerm(
                    Box::new(Expr::Var(e.clone())),
                    Box::new(Expr::Const(value)),
                ));
                e
            }
            Reifier::Nothing => {
                items.empty = true;
                return Ok(());
            }
        };
        let s = self.position(&tp.subject, view, items)?;
        let pred = match &tp.predicate {
            NamedNodePattern::NamedNode(n) => TermOrVar::Const(Value::Iri(n.as_str().to_string())),
            NamedNodePattern::Variable(v) => TermOrVar::Var(Var::new(v.as_str())),
        };
        let o = self.position(&tp.object, view, items)?;
        let mut t = TriplePattern::new(s, pred, o, view);
        t.eid = Some(eid);
        items.triples.push(t);
        Ok(())
    }
}

impl Lowerer<'_> {
    /// Takes the `tm:text*` patterns out of `triples` and turns each group on one
    /// subject variable into a text recall under `view` and the active graph
    /// selection (`lat.md/query#Text Recall`).
    fn text_patterns(&mut self, triples: &mut Vec<TriplePattern>, view: View) -> Result<Vec<Op>> {
        let is_text = |t: &TriplePattern| matches!(&t.p, TermOrVar::Const(Value::Iri(p)) if tm_ir::vocab::TEXT.contains(&p.as_str()));
        if !triples.iter().any(is_text) {
            return Ok(Vec::new());
        }
        let (text, rest): (Vec<TriplePattern>, Vec<TriplePattern>) =
            std::mem::take(triples).into_iter().partition(is_text);
        *triples = rest;
        let bad = |msg: &str| Error::invalid_query(format!("text recall: {msg}"));
        let mut groups: Vec<TextPattern> = Vec::new();
        for t in &text {
            let TermOrVar::Var(e) = &t.s else {
                return Err(bad("the subject of a tm:text pattern must be a variable"));
            };
            if !groups.iter().any(|g| &g.eid == e) {
                groups.push(TextPattern::new(
                    TermOrVar::Const(Value::Str(String::new())),
                    e.name(),
                    view,
                ));
            }
        }
        for g in &mut groups {
            let mut matched = false;
            for t in text.iter().filter(|t| t.s.as_var() == Some(&g.eid)) {
                let TermOrVar::Const(Value::Iri(p)) = &t.p else {
                    unreachable!("text patterns have constant predicates")
                };
                let out_var = |o: &TermOrVar| match o {
                    TermOrVar::Var(v) => Ok(Some(v.clone())),
                    _ => Err(bad(&format!("<{p}> binds a variable"))),
                };
                match p.as_str() {
                    tm_ir::vocab::TM_TEXT_MATCH => {
                        if matched {
                            return Err(bad("one tm:textMatch per subject"));
                        }
                        matched = true;
                        g.query = match &t.o {
                            TermOrVar::Const(Value::Str(s)) => {
                                TermOrVar::Const(Value::Str(s.clone()))
                            }
                            TermOrVar::Const(Value::LangStr { lex, .. }) => {
                                TermOrVar::Const(Value::Str(lex.clone()))
                            }
                            _ => return Err(bad("tm:textMatch needs a string literal")),
                        };
                    }
                    tm_ir::vocab::TM_TEXT_SCORE => g.score = out_var(&t.o)?,
                    tm_ir::vocab::TM_TEXT_RANK => g.rank = out_var(&t.o)?,
                    tm_ir::vocab::TM_TEXT_CONFIDENCE => g.confidence = out_var(&t.o)?,
                    tm_ir::vocab::TM_TEXT_LIMIT => {
                        g.limit = match &t.o {
                            TermOrVar::Const(Value::Int(n)) if *n >= 0 => {
                                Some(u32::try_from(*n).unwrap_or(u32::MAX))
                            }
                            _ => return Err(bad("tm:textLimit needs a non-negative integer")),
                        }
                    }
                    _ => {
                        g.mode = match &t.o {
                            TermOrVar::Const(Value::Str(s)) => tm_core::TextMode::from_name(s)
                                .ok_or_else(|| {
                                    bad("tm:textMode is \"all\", \"any\" or \"phrase\"")
                                })?,
                            _ => return Err(bad("tm:textMode needs a string literal")),
                        }
                    }
                }
            }
            if !matched {
                return Err(bad(&format!(
                    "{} has tm:text patterns but no tm:textMatch",
                    g.eid
                )));
            }
            g.graph = match &self.active {
                tm_ir::GraphSel::Var(_) => {
                    return Err(bad("GRAPH ?g around tm:textMatch (name the graphs)"))
                }
                other => other.clone(),
            };
        }
        Ok(groups.into_iter().map(Op::Text).collect())
    }
}

/// Drops a plain `(s, p, o)` pattern when the same list has an eid-bound pattern
/// with the same terms and view: the eid pattern implies the triple.
fn eliminate_redundant(triples: &mut Vec<TriplePattern>) {
    let bound: Vec<TriplePattern> = triples
        .iter()
        .filter(|t| t.eid.is_some())
        .cloned()
        .collect();
    triples.retain(|t| {
        t.eid.is_some()
            || !bound
                .iter()
                .any(|b| b.s == t.s && b.p == t.p && b.o == t.o && b.view == t.view)
    });
}
