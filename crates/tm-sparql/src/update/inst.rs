//! Template instantiation: one solution (or none, for data blocks) turns a
//! `TriplePattern` into a concrete triple over values.

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use tm_core::{Result, Value};

use crate::results::Solutions;
use crate::terms;

/// A position of an instantiated triple.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// A value (IRI, literal, statement, node, ...).
    Val(Value),
    /// A blank node label of the template or data block.
    Blank(String),
    /// A triple term `<<( s p o )>>`: the statement with that content.
    Triple(Box<TripleN>),
}

/// An instantiated triple.
#[derive(Clone, Debug, PartialEq)]
pub struct TripleN {
    /// Subject.
    pub s: Node,
    /// Predicate IRI.
    pub p: Value,
    /// Object.
    pub o: Node,
}

/// The solution row an instantiation reads variables from.
#[derive(Copy, Clone)]
pub struct Row<'a> {
    /// The solutions.
    pub sol: &'a Solutions,
    /// The row index.
    pub row: usize,
}

fn node(t: &TermPattern, row: Option<Row<'_>>) -> Result<Option<Node>> {
    Ok(match t {
        TermPattern::NamedNode(n) => Some(Node::Val(terms::named_node(n))),
        TermPattern::Literal(l) => Some(Node::Val(terms::literal(l)?)),
        TermPattern::BlankNode(b) => Some(Node::Blank(b.as_str().to_string())),
        TermPattern::Variable(v) => {
            row.and_then(|r| r.sol.get(r.row, v.as_str()).cloned().map(Node::Val))
        }
        TermPattern::Triple(tp) => instantiate(tp, row)?.map(|t| Node::Triple(Box::new(t))),
    })
}

fn subject_ok(n: &Node) -> bool {
    match n {
        Node::Val(v) => matches!(
            v,
            Value::Iri(_) | Value::Node(_) | Value::BNode(_) | Value::Stmt(_) | Value::Tx(_)
        ),
        Node::Blank(_) => true,
        Node::Triple(_) => false,
    }
}

/// The triple of `t` for `row`, or `None` when a variable is unbound or a term is
/// invalid in its position (a literal subject, a non-IRI predicate).
pub fn instantiate(t: &TriplePattern, row: Option<Row<'_>>) -> Result<Option<TripleN>> {
    let s = node(&t.subject, row)?;
    let p = match &t.predicate {
        NamedNodePattern::NamedNode(n) => Some(Value::Iri(n.as_str().to_string())),
        NamedNodePattern::Variable(v) => row
            .and_then(|r| r.sol.get(r.row, v.as_str()).cloned())
            .filter(|v| matches!(v, Value::Iri(_))),
    };
    let o = node(&t.object, row)?;
    Ok(match (s, p, o) {
        (Some(s), Some(p), Some(o)) if subject_ok(&s) => Some(TripleN { s, p, o }),
        _ => None,
    })
}
