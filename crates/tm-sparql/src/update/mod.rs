//! SPARQL Update (design D8): the request is checked and lowered before the
//! transaction opens; [`run`] executes it inside one `Db::transact`.

mod inst;
mod run;

use spargebra::term::{
    GraphName, GraphNamePattern, GroundQuad, GroundQuadPattern, GroundTerm, GroundTermPattern,
    GroundTriple, GroundTriplePattern, NamedNodePattern, Quad, QuadPattern, Term, TermPattern,
    Triple, TriplePattern,
};
use spargebra::{GraphUpdateOperation, Update};
use tm_core::Result;
use tm_ir::validate::scope;
use tm_ir::{IrQuery, Op, Project};

use crate::dataset::ViewScope;
use crate::env::Env;
use crate::error::{unsupported, CLEAR, CREATE, DROP, LOAD, NAMED_GRAPH, UPDATE_NON_CURRENT};
use crate::lower::vars::visible;
use crate::lower::Lowerer;
use crate::parse::AssocHints;

pub use inst::{Node, TripleN};
pub use run::run;

/// One lowered update operation.
#[derive(Clone, Debug)]
pub enum UpdateOp {
    /// `INSERT DATA`: ground triples (blank nodes become fresh blank nodes).
    InsertData(Vec<TriplePattern>),
    /// `DELETE DATA`: ground triples.
    DeleteData(Vec<TriplePattern>),
    /// `DELETE/INSERT … WHERE`, `DELETE WHERE`.
    DeleteInsert {
        /// The delete templates.
        delete: Vec<TriplePattern>,
        /// The insert templates.
        insert: Vec<TriplePattern>,
        /// The `WHERE` pattern as an IR query over its visible variables.
        select: Box<IrQuery>,
    },
}

/// A checked update request: the operations run in order in one transaction.
#[derive(Clone, Debug)]
pub struct UpdatePlan {
    /// The operations.
    pub ops: Vec<UpdateOp>,
}

fn not_named(g: &GraphName) -> Result<()> {
    match g {
        GraphName::DefaultGraph => Ok(()),
        GraphName::NamedNode(_) => Err(unsupported(NAMED_GRAPH)),
    }
}

fn not_named_pattern(g: &GraphNamePattern) -> Result<()> {
    match g {
        GraphNamePattern::DefaultGraph => Ok(()),
        _ => Err(unsupported(NAMED_GRAPH)),
    }
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

fn insert_quad(q: &Quad) -> Result<TriplePattern> {
    not_named(&q.graph_name)?;
    Ok(TriplePattern {
        subject: match &q.subject {
            spargebra::term::NamedOrBlankNode::NamedNode(n) => TermPattern::NamedNode(n.clone()),
            spargebra::term::NamedOrBlankNode::BlankNode(b) => TermPattern::BlankNode(b.clone()),
        },
        predicate: NamedNodePattern::NamedNode(q.predicate.clone()),
        object: term(&q.object),
    })
}

fn delete_quad(q: &GroundQuad) -> Result<TriplePattern> {
    not_named(&q.graph_name)?;
    Ok(TriplePattern {
        subject: TermPattern::NamedNode(q.subject.clone()),
        predicate: NamedNodePattern::NamedNode(q.predicate.clone()),
        object: ground_term(&q.object),
    })
}

fn insert_template(q: &QuadPattern) -> Result<TriplePattern> {
    not_named_pattern(&q.graph_name)?;
    Ok(TriplePattern {
        subject: q.subject.clone(),
        predicate: q.predicate.clone(),
        object: q.object.clone(),
    })
}

fn delete_template(q: &GroundQuadPattern) -> Result<TriplePattern> {
    not_named_pattern(&q.graph_name)?;
    Ok(TriplePattern {
        subject: ground_pattern_term(&q.subject),
        predicate: q.predicate.clone(),
        object: ground_pattern_term(&q.object),
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

/// Checks and lowers an update request. Nothing is executed: every unsupported
/// operation and every `GRAPH` in data or templates fails here, before any
/// transaction opens.
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
                let delete: Vec<TriplePattern> =
                    delete.iter().map(delete_template).collect::<Result<_>>()?;
                let insert: Vec<TriplePattern> =
                    insert.iter().map(insert_template).collect::<Result<_>>()?;
                let sc = ViewScope::from_dataset(using.as_ref())?;
                let mut l = Lowerer::new(env);
                l.assoc = assoc;
                let op = l.pattern(pattern, sc)?;
                UpdateOp::DeleteInsert {
                    delete,
                    insert,
                    select: Box::new(select_of(op)),
                }
            }
            GraphUpdateOperation::Load { .. } => return Err(unsupported(LOAD)),
            GraphUpdateOperation::Clear { .. } => return Err(unsupported(CLEAR)),
            GraphUpdateOperation::Create { .. } => return Err(unsupported(CREATE)),
            GraphUpdateOperation::Drop { .. } => return Err(unsupported(DROP)),
        });
    }
    Ok(UpdatePlan { ops })
}
