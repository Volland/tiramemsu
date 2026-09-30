//! SPARQL Update (design D8): the request is checked and lowered before the
//! transaction opens; [`run`] executes it inside one `Db::transact`.

mod inst;
mod run;

use spargebra::algebra::GraphTarget as AlgGraphTarget;
use spargebra::term::{
    GraphName, GraphNamePattern, GroundQuad, GroundQuadPattern, GroundTerm, GroundTermPattern,
    GroundTriple, GroundTriplePattern, NamedNode, NamedNodePattern, Quad, QuadPattern, Term,
    TermPattern, Triple, TriplePattern,
};
use spargebra::{GraphUpdateOperation, Update};
use tm_core::{Result, Value};
use tm_ir::validate::scope;
use tm_ir::{IrQuery, Op, Project};

use crate::dataset::{graph_name, is_tm_iri, GraphDataset, ViewScope};
use crate::env::Env;
use crate::error::{
    parse_error, time_iri_in_graph, unsupported, CLEAR, DROP, LOAD, UPDATE_NON_CURRENT,
};
use crate::lower::vars::visible;
use crate::lower::Lowerer;
use crate::parse::AssocHints;

pub use inst::{Node, TripleN};
pub use run::run;

/// The graph of a template triple.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphRef {
    /// No `GRAPH` block: the plain statement, no membership.
    Default,
    /// `GRAPH <g>`: the statement and a membership in `g`.
    Named(Value),
    /// `GRAPH ?g`: the graph the `WHERE` solution binds to `?g`.
    Var(String),
}

/// A template or data triple with its graph.
#[derive(Clone, Debug)]
pub struct Template {
    /// The triple pattern.
    pub triple: TriplePattern,
    /// Where it is written.
    pub graph: GraphRef,
}

/// The target of a `CLEAR` or `DROP` that is supported.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphTarget {
    /// `GRAPH <g>`.
    Graph(Value),
    /// `NAMED`: every graph that has a live membership or a declaration.
    Named,
}

/// One lowered update operation.
#[derive(Clone, Debug)]
pub enum UpdateOp {
    /// `INSERT DATA`: ground triples (blank nodes become fresh blank nodes).
    InsertData(Vec<Template>),
    /// `DELETE DATA`: ground triples.
    DeleteData(Vec<Template>),
    /// `DELETE/INSERT … WHERE`, `DELETE WHERE`.
    DeleteInsert {
        /// The delete templates.
        delete: Vec<Template>,
        /// The insert templates.
        insert: Vec<Template>,
        /// The `WHERE` pattern as an IR query over its visible variables.
        select: Box<IrQuery>,
    },
    /// `CREATE GRAPH`.
    Create {
        /// The graph.
        graph: Value,
        /// `SILENT`.
        silent: bool,
    },
    /// `CLEAR` (`drop = false`) or `DROP` (`drop = true`).
    Clear {
        /// What to clear.
        target: GraphTarget,
        /// `SILENT`.
        silent: bool,
        /// `DROP` also retracts the `sys:Graph` declaration.
        drop: bool,
    },
}

/// A checked update request: the operations run in order in one transaction.
///
/// Produced by [`prepare`](crate::prepare) (as [`Prepared::Update`](crate::Prepared::Update))
/// before any transaction opens, so unsupported operations fail without side
/// effects. Insert maps to assert, delete to retract with cascade to layers.
/// Execute it with [`run`] inside a `tm_core::Tx`; the facade does this in one
/// transaction per request.
#[derive(Clone, Debug)]
pub struct UpdatePlan {
    /// The operations.
    pub ops: Vec<UpdateOp>,
}

/// A graph name of a `GRAPH` block, `CREATE`, `CLEAR` or `DROP`: a `tm:` IRI is a
/// `Parse` error naming `SERVICE`, a statement or transaction IRI is
/// `InvalidGraphName`.
fn graph_iri(n: &NamedNode) -> Result<Value> {
    if is_tm_iri(n.as_str()) {
        return Err(time_iri_in_graph(n.as_str()));
    }
    graph_name(n.as_str())
}

fn quad_graph(g: &GraphName) -> Result<GraphRef> {
    Ok(match g {
        GraphName::DefaultGraph => GraphRef::Default,
        GraphName::NamedNode(n) => GraphRef::Named(graph_iri(n)?),
    })
}

fn pattern_graph(g: &GraphNamePattern) -> Result<GraphRef> {
    Ok(match g {
        GraphNamePattern::DefaultGraph => GraphRef::Default,
        GraphNamePattern::NamedNode(n) => GraphRef::Named(graph_iri(n)?),
        GraphNamePattern::Variable(v) => GraphRef::Var(v.as_str().to_string()),
    })
}

fn term(t: &Term) -> TermPattern {
    match t {
        Term::NamedNode(n) => TermPattern::NamedNode(n.clone()),
        Term::BlankNode(b) => TermPattern::BlankNode(b.clone()),
        Term::Literal(l) => TermPattern::Literal(l.clone()),
        Term::Triple(t) => TermPattern::Triple(Box::new(triple(t))),
    }
}

fn triple(t: &Triple) -> TriplePattern {
    TriplePattern {
        subject: match &t.subject {
            spargebra::term::NamedOrBlankNode::NamedNode(n) => TermPattern::NamedNode(n.clone()),
            spargebra::term::NamedOrBlankNode::BlankNode(b) => TermPattern::BlankNode(b.clone()),
        },
        predicate: NamedNodePattern::NamedNode(t.predicate.clone()),
        object: term(&t.object),
    }
}

fn ground_term(t: &GroundTerm) -> TermPattern {
    match t {
        GroundTerm::NamedNode(n) => TermPattern::NamedNode(n.clone()),
        GroundTerm::Literal(l) => TermPattern::Literal(l.clone()),
        GroundTerm::Triple(t) => TermPattern::Triple(Box::new(ground_triple(t))),
    }
}

fn ground_triple(t: &GroundTriple) -> TriplePattern {
    TriplePattern {
        subject: TermPattern::NamedNode(t.subject.clone()),
        predicate: NamedNodePattern::NamedNode(t.predicate.clone()),
        object: ground_term(&t.object),
    }
}

fn ground_pattern_term(t: &GroundTermPattern) -> TermPattern {
    match t {
        GroundTermPattern::NamedNode(n) => TermPattern::NamedNode(n.clone()),
        GroundTermPattern::Literal(l) => TermPattern::Literal(l.clone()),
        GroundTermPattern::Variable(v) => TermPattern::Variable(v.clone()),
        GroundTermPattern::Triple(t) => TermPattern::Triple(Box::new(ground_triple_pattern(t))),
    }
}

fn ground_triple_pattern(t: &GroundTriplePattern) -> TriplePattern {
    TriplePattern {
        subject: ground_pattern_term(&t.subject),
        predicate: t.predicate.clone(),
        object: ground_pattern_term(&t.object),
    }
}

fn insert_quad(q: &Quad) -> Result<Template> {
    Ok(Template {
        graph: quad_graph(&q.graph_name)?,
        triple: TriplePattern {
            subject: match &q.subject {
                spargebra::term::NamedOrBlankNode::NamedNode(n) => {
                    TermPattern::NamedNode(n.clone())
                }
                spargebra::term::NamedOrBlankNode::BlankNode(b) => {
                    TermPattern::BlankNode(b.clone())
                }
            },
            predicate: NamedNodePattern::NamedNode(q.predicate.clone()),
            object: term(&q.object),
        },
    })
}

fn delete_quad(q: &GroundQuad) -> Result<Template> {
    Ok(Template {
        graph: quad_graph(&q.graph_name)?,
        triple: TriplePattern {
            subject: TermPattern::NamedNode(q.subject.clone()),
            predicate: NamedNodePattern::NamedNode(q.predicate.clone()),
            object: ground_term(&q.object),
        },
    })
}

fn insert_template(q: &QuadPattern) -> Result<Template> {
    Ok(Template {
        graph: pattern_graph(&q.graph_name)?,
        triple: TriplePattern {
            subject: q.subject.clone(),
            predicate: q.predicate.clone(),
            object: q.object.clone(),
        },
    })
}

fn delete_template(q: &GroundQuadPattern) -> Result<Template> {
    Ok(Template {
        graph: pattern_graph(&q.graph_name)?,
        triple: TriplePattern {
            subject: ground_pattern_term(&q.subject),
            predicate: q.predicate.clone(),
            object: ground_pattern_term(&q.object),
        },
    })
}

fn select_of(op: Op) -> IrQuery {
    let mut vars = visible(&scope(&op).vars);
    let mut op = op;
    if vars.is_empty() {
        // a pattern without variables: keep one column so that the query has rows
        let flag = tm_ir::Var::new(format!("{}row", crate::lower::vars::INTERNAL_PREFIX));
        op = op.extend(flag.name(), tm_ir::Expr::Const(tm_core::Value::Bool(true)));
        vars = vec![flag];
    }
    IrQuery::sparql(Op::Project(Project {
        input: Box::new(op),
        vars,
        distinct: false,
    }))
}

/// The graph variables of `templates` that the `WHERE` pattern does not bind.
fn check_graph_vars(templates: &[Template], select: &IrQuery) -> Result<()> {
    let bound = scope(&select.root).vars;
    for t in templates {
        if let GraphRef::Var(v) = &t.graph {
            if !bound.iter().any(|b| b.name() == v) {
                return Err(parse_error(
                    None,
                    format!("graph variable ?{v} in a template is not bound by the WHERE pattern"),
                ));
            }
        }
    }
    Ok(())
}

/// Checks and lowers an update request. Nothing is executed: every unsupported
/// operation and every invalid graph name fails here, before any transaction opens.
pub fn plan_update(u: &Update, env: &Env, assoc: AssocHints) -> Result<UpdatePlan> {
    if !env.is_current() {
        return Err(unsupported(UPDATE_NON_CURRENT));
    }
    let mut ops = Vec::new();
    for op in &u.operations {
        ops.push(match op {
            GraphUpdateOperation::InsertData { data } => {
                UpdateOp::InsertData(data.iter().map(insert_quad).collect::<Result<_>>()?)
            }
            GraphUpdateOperation::DeleteData { data } => {
                UpdateOp::DeleteData(data.iter().map(delete_quad).collect::<Result<_>>()?)
            }
            GraphUpdateOperation::DeleteInsert {
                delete,
                insert,
                using,
                pattern,
            } => {
                let delete: Vec<Template> =
                    delete.iter().map(delete_template).collect::<Result<_>>()?;
                let insert: Vec<Template> =
                    insert.iter().map(insert_template).collect::<Result<_>>()?;
                let sc = ViewScope::from_dataset(using.as_ref())?;
                let mut l = Lowerer::new(env);
                l.assoc = assoc;
                l.set_dataset(GraphDataset::from_dataset(using.as_ref())?);
                let op = l.pattern(pattern, sc)?;
                let select = select_of(op);
                check_graph_vars(&delete, &select)?;
                check_graph_vars(&insert, &select)?;
                UpdateOp::DeleteInsert {
                    delete,
                    insert,
                    select: Box::new(select),
                }
            }
            GraphUpdateOperation::Load { .. } => return Err(unsupported(LOAD)),
            GraphUpdateOperation::Create { silent, graph } => UpdateOp::Create {
                graph: graph_iri(graph)?,
                silent: *silent,
            },
            GraphUpdateOperation::Clear { silent, graph } => UpdateOp::Clear {
                target: graph_target(graph, CLEAR)?,
                silent: *silent,
                drop: false,
            },
            GraphUpdateOperation::Drop { silent, graph } => UpdateOp::Clear {
                target: graph_target(graph, DROP)?,
                silent: *silent,
                drop: true,
            },
        });
    }
    Ok(UpdatePlan { ops })
}

/// The supported targets of `CLEAR` / `DROP`; `DEFAULT` and `ALL` are not.
fn graph_target(g: &AlgGraphTarget, keyword: &str) -> Result<GraphTarget> {
    match g {
        AlgGraphTarget::NamedNode(n) => Ok(GraphTarget::Graph(graph_iri(n)?)),
        AlgGraphTarget::NamedGraphs => Ok(GraphTarget::Named),
        AlgGraphTarget::DefaultGraph => Err(unsupported(format!("{keyword} DEFAULT"))),
        AlgGraphTarget::AllGraphs => Err(unsupported(format!("{keyword} ALL"))),
    }
}
