//! Running a checked update inside one transaction.

use std::collections::HashMap;

use tm_core::vector::{parse_vector, EmbeddingModel, Metric, VectorEncoding};
use tm_core::{vocab, AssertOpts, Eid, Error, ObjectId, Result, SqlValue, Tx, Valid, Value};
use tm_ir::IrQuery;

use super::inst::{instantiate, Node, Row, TripleN};
use super::{vector_kind, GraphRef, GraphTarget, Template, UpdateOp, UpdatePlan, VectorKind};
use crate::error::{unsupported, REIFIER_MANY, REIFIER_NOT_STATEMENT, REIFIES_WITHOUT_TRIPLE};
use crate::results::Solutions;

const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// Evaluates an IR query on the writer connection of the running transaction.
pub type Select<'a> = dyn FnMut(&mut Tx<'_>, &IrQuery) -> Result<Solutions> + 'a;

/// A triple with the graph it is written to (`None`: no `GRAPH` block).
type InGraph = (TripleN, Option<Value>);

/// Runs every operation of `plan` in order on `tx`. The `WHERE` of an operation
/// is evaluated through `select` on the transaction's own connection, so it sees
/// the effects of the earlier operations.
pub fn run(plan: &UpdatePlan, tx: &mut Tx<'_>, select: &mut Select<'_>) -> Result<()> {
    for op in &plan.ops {
        match op {
            UpdateOp::InsertData(templates) => {
                let ts = instantiate_all(templates, None)?;
                Inserter::new(tx).insert_all(ts)?;
            }
            UpdateOp::DeleteData(templates) => {
                let ts = instantiate_all(templates, None)?;
                delete(tx, ts)?;
            }
            UpdateOp::DeleteInsert {
                delete: del,
                insert,
                select: q,
            } => {
                // the WHERE sees the state before this operation
                let sol = select(tx, q)?;
                let mut deletes = Vec::new();
                for row in 0..sol.rows.len() {
                    let r = Row { sol: &sol, row };
                    deletes.extend(instantiate_all(del, Some(r))?);
                }
                delete(tx, deletes)?;
                for row in 0..sol.rows.len() {
                    let r = Row { sol: &sol, row };
                    let ts = instantiate_all(insert, Some(r))?;
                    // blank nodes are fresh for each solution
                    Inserter::new(tx).insert_all(ts)?;
                }
            }
            UpdateOp::Create { graph, silent } => {
                if declared(tx, graph)? {
                    if !silent {
                        return Err(Error::GraphExists {
                            graph: graph.to_string(),
                        });
                    }
                } else {
                    tx.create_graph(graph)?;
                }
            }
            UpdateOp::Clear {
                target,
                silent,
                drop,
            } => clear(tx, target, *silent, *drop)?,
        }
    }
    Ok(())
}

/// The graph of `v` if it is already known to the store (`None`: no statement
/// mentions it, so it has no membership and no declaration). A value that cannot
/// name a graph fails with `InvalidGraphName`.
fn known_graph(tx: &mut Tx<'_>, v: &Value) -> Result<Option<ObjectId>> {
    if !matches!(
        v,
        Value::Iri(_) | Value::Node(_) | Value::BNode(_) | Value::Stmt(_)
    ) {
        return Err(Error::InvalidGraphName {
            term: v.to_string(),
        });
    }
    tx.lookup(v)
}

fn declared(tx: &mut Tx<'_>, g: &Value) -> Result<bool> {
    match known_graph(tx, g)? {
        Some(id) => tx.graph_declared(id),
        None => Ok(false),
    }
}

/// `CLEAR` and `DROP` of one graph or of every named graph.
fn clear(tx: &mut Tx<'_>, target: &GraphTarget, silent: bool, drop: bool) -> Result<()> {
    let graphs: Vec<ObjectId> = match target {
        GraphTarget::Named => tx.live_graphs()?,
        GraphTarget::Graph(g) => {
            let id = known_graph(tx, g)?;
            let exists = match id {
                Some(id) => tx.graph_declared(id)? || tx.graph_has_members(id)?,
                None => false,
            };
            match (exists, id) {
                (true, Some(id)) => vec![id],
                _ if silent => Vec::new(),
                _ => {
                    return Err(Error::GraphNotFound {
                        graph: g.to_string(),
                    })
                }
            }
        }
    };
    for g in graphs {
        if drop {
            tx.drop_graph(g)?;
        } else {
            tx.clear_graph(g)?;
        }
    }
    Ok(())
}

fn instantiate_all(templates: &[Template], row: Option<Row<'_>>) -> Result<Vec<InGraph>> {
    let mut out = Vec::new();
    for t in templates {
        let graph = match &t.graph {
            GraphRef::Default => None,
            GraphRef::Named(g) => Some(g.clone()),
            GraphRef::Var(v) => match row.and_then(|r| r.sol.get(r.row, v).cloned()) {
                Some(g) => Some(g),
                // an unbound graph variable drops the triple, like any unbound variable
                None => continue,
            },
        };
        if let Some(triple) = instantiate(&t.triple, row)? {
            out.push((triple, graph));
        }
    }
    Ok(out)
}

fn is_reifies(t: &TripleN) -> bool {
    matches!(&t.p, Value::Iri(p) if p == RDF_REIFIES)
}

/// Who a reifier is.
#[derive(Clone, Debug, PartialEq)]
enum Key {
    Blank(String),
    Stmt(Eid),
}

fn reifier_key(n: &Node) -> Result<Key> {
    match n {
        Node::Blank(l) => Ok(Key::Blank(l.clone())),
        Node::Val(Value::Stmt(e)) => Ok(Key::Stmt(*e)),
        _ => Err(unsupported(REIFIER_NOT_STATEMENT)),
    }
}

struct Inserter<'a, 't> {
    tx: &'a mut Tx<'t>,
    bnodes: HashMap<String, ObjectId>,
    sigma: HashMap<String, Eid>,
}

impl<'a, 't> Inserter<'a, 't> {
    fn new(tx: &'a mut Tx<'t>) -> Self {
        Inserter {
            tx,
            bnodes: HashMap::new(),
            sigma: HashMap::new(),
        }
    }

    fn resolve(&mut self, n: &Node) -> Result<ObjectId> {
        match n {
            Node::Val(v) => self.tx.encode(v),
            Node::Blank(l) => {
                if let Some(e) = self.sigma.get(l) {
                    return Ok(e.oid());
                }
                if let Some(id) = self.bnodes.get(l) {
                    return Ok(*id);
                }
                let id = self.tx.new_bnode()?;
                self.bnodes.insert(l.clone(), id);
                Ok(id)
            }
            Node::Triple(t) => Ok(self.assert_triple(t)?.oid()),
        }
    }

    fn assert_triple(&mut self, t: &TripleN) -> Result<Eid> {
        let s = self.resolve(&t.s)?;
        let p = self.tx.encode(&t.p)?;
        let o = self.resolve(&t.o)?;
        Ok(self.tx.assert(s, p, o, Valid::ALWAYS)?.eid())
    }

    /// Adds statement `eid` to `graph` (a membership, idempotent).
    fn add_member(&mut self, eid: Eid, graph: &Option<Value>) -> Result<()> {
        if let Some(g) = graph {
            self.tx.add_to_graph(eid, g, AssertOpts::default())?;
        }
        Ok(())
    }

    /// Inserts `all`: plain triples first (binding reifiers), then the model
    /// definitions and embeddings among them, which are engine operations rather
    /// than triples (`lat.md/query#Vector Recall#Writes From Query Languages`).
    fn insert_all(&mut self, all: Vec<InGraph>) -> Result<()> {
        let (vectors, rest): (Vec<InGraph>, Vec<InGraph>) =
            all.into_iter().partition(|(t, _)| kind_of(t).is_some());
        self.insert(rest)?;
        for (subject, group) in groups(&vectors) {
            let kinds: Vec<VectorKind> = group.iter().filter_map(|t| kind_of(t)).collect();
            if kinds.contains(&VectorKind::Definition) && kinds.contains(&VectorKind::Embedding) {
                return Err(Error::invalid_query(
                    "one subject cannot be both an embedding model and an embedded statement",
                ));
            }
            if kinds[0] == VectorKind::Definition {
                self.define(subject, &group)?;
            }
        }
        for (subject, group) in groups(&vectors) {
            if kind_of(group[0]) == Some(VectorKind::Embedding) {
                self.embed(subject, &group)?;
            }
        }
        Ok(())
    }

    /// One model definition from its `sys:` triples on `subject`.
    fn define(&mut self, subject: &Node, group: &[&TripleN]) -> Result<()> {
        let Node::Val(model) = subject else {
            return Err(Error::invalid_query(
                "an embedding model is named by an IRI",
            ));
        };
        let mut def = EmbeddingModel::new(0);
        let (dims, metric, enc) = (
            one(group, vocab::SYS_DIMENSIONS)?,
            one(group, vocab::SYS_METRIC)?,
            one(group, vocab::SYS_VECTOR_ENCODING)?,
        );
        def.dimensions = match dims {
            Some(Node::Val(Value::Int(n))) => u32::try_from(*n)
                .map_err(|_| Error::invalid_query("sys:dimensions is out of range"))?,
            Some(_) => return Err(Error::invalid_query("sys:dimensions needs an integer")),
            None => {
                return Err(Error::invalid_query(format!(
                    "the definition of {model} needs sys:dimensions"
                )))
            }
        };
        if let Some(m) = metric {
            def.metric = match m {
                Node::Val(Value::Str(s)) => Metric::from_name(s),
                _ => None,
            }
            .ok_or_else(|| Error::invalid_query(r#"sys:metric needs "cosine", "dot" or "l2""#))?;
        }
        if let Some(e) = enc {
            def.encoding = match e {
                Node::Val(Value::Str(s)) => VectorEncoding::from_name(s),
                _ => None,
            }
            .ok_or_else(|| Error::invalid_query(r#"sys:vectorEncoding needs "f32" or "f16""#))?;
        }
        self.tx.define_embedding_model(model, &def)?;
        Ok(())
    }

    /// One embedding from the `tm:vectorEmbedding` and `tm:vectorModel` triples on
    /// `subject`, which must denote a statement.
    fn embed(&mut self, subject: &Node, group: &[&TripleN]) -> Result<()> {
        let eid = match subject {
            Node::Val(Value::Stmt(e)) => Some(*e),
            Node::Blank(l) => self.sigma.get(l).copied(),
            _ => None,
        }
        .ok_or_else(|| {
            Error::invalid_query(
                "the subject of tm:vectorEmbedding must denote a statement \
                 (a statement IRI, a bound ?e, or a reifier of an inserted triple)",
            )
        })?;
        let vector = match one(group, tm_ir::vocab::TM_VECTOR_EMBEDDING)? {
            Some(Node::Val(Value::Str(s))) => parse_vector(s)
                .map_err(|m| Error::invalid_query(format!("tm:vectorEmbedding: {m}")))?,
            Some(_) => {
                return Err(Error::invalid_query(
                    "tm:vectorEmbedding needs a string literal holding a JSON array",
                ))
            }
            None => {
                return Err(Error::invalid_query(
                    "tm:vectorModel without tm:vectorEmbedding",
                ))
            }
        };
        let model = match one(group, tm_ir::vocab::TM_VECTOR_MODEL)? {
            Some(Node::Val(m @ Value::Iri(_))) => m.clone(),
            Some(_) => return Err(Error::invalid_query("tm:vectorModel needs an IRI")),
            None => {
                return Err(Error::invalid_query(
                    "tm:vectorEmbedding needs tm:vectorModel",
                ))
            }
        };
        self.tx.set_embedding(eid, &model, &vector)?;
        Ok(())
    }

    fn insert(&mut self, all: Vec<InGraph>) -> Result<()> {
        let triples: Vec<&TripleN> = all.iter().map(|(t, _)| t).collect();
        // 1. reifiers: each maps to exactly one triple (and the graph of the pattern)
        let mut reifs: Vec<(Key, TripleN, Option<Value>)> = Vec::new();
        for (t, g) in all.iter().filter(|(t, _)| is_reifies(t)) {
            let Node::Triple(inner) = &t.o else {
                return Err(unsupported(REIFIES_WITHOUT_TRIPLE));
            };
            let key = reifier_key(&t.s)?;
            match reifs.iter().find(|(k, _, _)| *k == key) {
                Some((_, prev, _)) if prev != &**inner => return Err(unsupported(REIFIER_MANY)),
                Some(_) => {}
                None => reifs.push((key, (**inner).clone(), g.clone())),
            }
        }
        // 2. assert the reified statements; bind blank reifiers to their eids
        for (key, triple, graph) in &reifs {
            let asserted = self.assert_triple(triple)?;
            self.add_member(asserted, graph)?;
            match key {
                Key::Blank(l) => {
                    self.sigma.insert(l.clone(), asserted);
                }
                Key::Stmt(want) => {
                    let (s, p, o) = (
                        self.resolve(&triple.s)?,
                        self.tx.encode(&triple.p)?,
                        self.resolve(&triple.o)?,
                    );
                    if stored_content(self.tx, *want)? != Some([s.raw(), p.raw(), o.raw()]) {
                        return Err(unsupported(REIFIER_NOT_STATEMENT));
                    }
                }
            }
        }
        // 3. everything else
        for (t, g) in all.iter().filter(|(t, _)| !is_reifies(t)) {
            let eid = self.assert_triple(t)?;
            self.add_member(eid, g)?;
        }
        let _ = triples;
        Ok(())
    }
}

fn kind_of(t: &TripleN) -> Option<VectorKind> {
    match &t.p {
        Value::Iri(p) => vector_kind(p),
        _ => None,
    }
}

/// The vector triples grouped by subject, in first-seen order.
fn groups(ts: &[InGraph]) -> Vec<(&Node, Vec<&TripleN>)> {
    let mut out: Vec<(&Node, Vec<&TripleN>)> = Vec::new();
    for (t, _) in ts {
        match out.iter_mut().find(|(s, _)| **s == t.s) {
            Some((_, g)) => {
                if !g.contains(&t) {
                    g.push(t)
                }
            }
            None => out.push((&t.s, vec![t])),
        }
    }
    out
}

/// The single object of predicate `p` in `group` (`None` when absent); two
/// different objects are `InvalidQuery`.
fn one<'g>(group: &[&'g TripleN], p: &str) -> Result<Option<&'g Node>> {
    let mut found: Option<&Node> = None;
    for t in group
        .iter()
        .filter(|t| matches!(&t.p, Value::Iri(x) if x == p))
    {
        match found {
            Some(prev) if *prev != t.o => {
                return Err(Error::invalid_query(format!(
                    "more than one <{p}> on one subject"
                )))
            }
            _ => found = Some(&t.o),
        }
    }
    Ok(found)
}

/// Drops the models whose live definition statements the delete triples match;
/// a triple that matches no live statement is a no-op, like any absent triple.
fn drop_models(tx: &mut Tx<'_>, defs: &[InGraph]) -> Result<()> {
    for (t, _) in defs {
        let (Node::Val(s), Node::Val(o)) = (&t.s, &t.o) else {
            continue;
        };
        let (Some(si), Some(pi), Some(oi)) = (tx.lookup(s)?, tx.lookup(&t.p)?, tx.lookup(o)?)
        else {
            continue;
        };
        if !live_eids(tx, si, pi, oi)?.is_empty() {
            tx.drop_embedding_model(s)?;
        }
    }
    Ok(())
}

/// The `(s, p, o)` ids of a stored statement, live or not.
fn stored_content(tx: &mut Tx<'_>, e: Eid) -> Result<Option<[i64; 3]>> {
    tx.read_with(|exec| {
        Ok(exec
            .first_row(
                "SELECT s, p, o FROM triple WHERE eid = ?1",
                &[SqlValue::Integer(e.oid().raw())],
            )?
            .and_then(|r| Some([r[0].as_i64()?, r[1].as_i64()?, r[2].as_i64()?])))
    })
}

/// The live statements with content `(s, p, o)`.
fn live_eids(tx: &mut Tx<'_>, s: ObjectId, p: ObjectId, o: ObjectId) -> Result<Vec<ObjectId>> {
    tx.read_with(|exec| {
        Ok(exec
            .rows(
                "SELECT eid FROM triple WHERE t_ret IS NULL AND s = ?1 AND p = ?2 AND o = ?3 ORDER BY eid",
                &[
                    SqlValue::Integer(s.raw()),
                    SqlValue::Integer(p.raw()),
                    SqlValue::Integer(o.raw()),
                ],
            )?
            .into_iter()
            .filter_map(|r| r[0].as_i64().map(ObjectId::from_raw))
            .collect())
    })
}

/// The ids a delete-template position can denote (none for an unknown value).
fn candidate_ids(tx: &mut Tx<'_>, n: &Node) -> Result<Vec<ObjectId>> {
    match n {
        Node::Val(v) => Ok(tx.lookup(v)?.into_iter().collect()),
        Node::Blank(_) => Ok(Vec::new()),
        Node::Triple(t) => {
            let Some(p) = tx.lookup(&t.p)? else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for s in candidate_ids(tx, &t.s)? {
                for o in candidate_ids(tx, &t.o)? {
                    out.extend(live_eids(tx, s, p, o)?);
                }
            }
            Ok(out)
        }
    }
}

/// Retracts the statements the triples denote. A triple with a reifier bound to
/// an eid retracts exactly that eid; other triples retract every live statement
/// with that content. A triple in a `GRAPH` block retracts only the membership of
/// those statements in that graph. Instantiated triples are deduplicated.
fn delete(tx: &mut Tx<'_>, triples: Vec<InGraph>) -> Result<()> {
    let (defs, triples): (Vec<InGraph>, Vec<InGraph>) = triples
        .into_iter()
        .partition(|(t, _)| kind_of(t) == Some(VectorKind::Definition));
    drop_models(tx, &defs)?;
    let mut seen = std::collections::HashSet::new();
    let triples: Vec<InGraph> = triples
        .into_iter()
        .filter(|t| seen.insert(format!("{t:?}")))
        .collect();
    let mut covered: Vec<&TripleN> = Vec::new();
    for (t, graph) in triples.iter().filter(|(t, _)| is_reifies(t)) {
        let Node::Triple(inner) = &t.o else {
            return Err(unsupported(REIFIES_WITHOUT_TRIPLE));
        };
        let Key::Stmt(e) = reifier_key(&t.s)? else {
            return Err(unsupported(REIFIER_NOT_STATEMENT));
        };
        // the reifier must be a stored statement with exactly this content
        let want = match (
            candidate_ids(tx, &inner.s)?.first(),
            tx.lookup(&inner.p)?,
            candidate_ids(tx, &inner.o)?.first(),
        ) {
            (Some(s), Some(p), Some(o)) => Some([s.raw(), p.raw(), o.raw()]),
            _ => None,
        };
        if want.is_none() || stored_content(tx, e)? != want {
            return Err(unsupported(REIFIER_NOT_STATEMENT));
        }
        match graph {
            None => {
                tx.retract(e)?;
            }
            Some(g) => remove_member(tx, e, g)?,
        }
        covered.push(&**inner);
    }
    for (t, graph) in triples
        .iter()
        .filter(|(t, _)| !is_reifies(t) && !covered.contains(&t))
    {
        let Some(p) = tx.lookup(&t.p)? else {
            if let Some(g) = graph {
                known_graph(tx, g)?;
            }
            continue;
        };
        for s in candidate_ids(tx, &t.s)? {
            for o in candidate_ids(tx, &t.o)? {
                match graph {
                    None => {
                        tx.retract_matching(Some(s), Some(p), Some(o))?;
                    }
                    Some(g) => {
                        for e in live_eids(tx, s, p, o)? {
                            if let Some(e) = Eid::from_oid(e) {
                                remove_member(tx, e, g)?;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Removes statement `e` from graph `g`; a graph the store has never seen holds
/// nothing, so it is a no-op (and interns no term).
fn remove_member(tx: &mut Tx<'_>, e: Eid, g: &Value) -> Result<()> {
    if let Some(id) = known_graph(tx, g)? {
        tx.remove_from_graph(e, id)?;
    }
    Ok(())
}
