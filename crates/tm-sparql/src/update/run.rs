//! Running a checked update inside one transaction.

use std::collections::HashMap;

use tm_core::{Eid, ObjectId, Result, SqlValue, Tx, Valid, Value};
use tm_ir::IrQuery;

use super::inst::{instantiate, Node, Row, TripleN};
use super::{UpdateOp, UpdatePlan};
use crate::error::{unsupported, REIFIER_MANY, REIFIER_NOT_STATEMENT, REIFIES_WITHOUT_TRIPLE};
use crate::results::Solutions;

const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// Evaluates an IR query on the writer connection of the running transaction.
pub type Select<'a> = dyn FnMut(&mut Tx<'_>, &IrQuery) -> Result<Solutions> + 'a;

/// Runs every operation of `plan` in order on `tx`. The `WHERE` of an operation
/// is evaluated through `select` on the transaction's own connection, so it sees
/// the effects of the earlier operations.
pub fn run(plan: &UpdatePlan, tx: &mut Tx<'_>, select: &mut Select<'_>) -> Result<()> {
    for op in &plan.ops {
        match op {
            UpdateOp::InsertData(triples) => {
                let ts = instantiate_all(triples, None)?;
                Inserter::new(tx).insert(ts)?;
            }
            UpdateOp::DeleteData(triples) => {
                let ts = instantiate_all(triples, None)?;
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
                    Inserter::new(tx).insert(ts)?;
                }
            }
        }
    }
    Ok(())
}

fn instantiate_all(
    templates: &[spargebra::term::TriplePattern],
    row: Option<Row<'_>>,
) -> Result<Vec<TripleN>> {
    let mut out = Vec::new();
    for t in templates {
        if let Some(t) = instantiate(t, row)? {
            out.push(t);
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

    fn insert(&mut self, triples: Vec<TripleN>) -> Result<()> {
        // 1. reifiers: each maps to exactly one triple
        let mut reifs: Vec<(Key, TripleN)> = Vec::new();
        for t in triples.iter().filter(|t| is_reifies(t)) {
            let Node::Triple(inner) = &t.o else {
                return Err(unsupported(REIFIES_WITHOUT_TRIPLE));
            };
            let key = reifier_key(&t.s)?;
            match reifs.iter().find(|(k, _)| *k == key) {
                Some((_, prev)) if prev != &**inner => return Err(unsupported(REIFIER_MANY)),
                Some(_) => {}
                None => reifs.push((key, (**inner).clone())),
            }
        }
        // 2. assert the reified statements; bind blank reifiers to their eids
        for (key, triple) in &reifs {
            let asserted = self.assert_triple(triple)?;
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
        for t in triples.iter().filter(|t| !is_reifies(t)) {
            self.assert_triple(t)?;
        }
        Ok(())
    }
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
/// with that content. Instantiated triples are deduplicated.
fn delete(tx: &mut Tx<'_>, triples: Vec<TripleN>) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let triples: Vec<TripleN> = triples
        .into_iter()
        .filter(|t| seen.insert(format!("{t:?}")))
        .collect();
    let mut covered: Vec<TripleN> = Vec::new();
    for t in triples.iter().filter(|t| is_reifies(t)) {
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
        tx.retract(e)?;
        covered.push((**inner).clone());
    }
    for t in triples
        .iter()
        .filter(|t| !is_reifies(t) && !covered.contains(t))
    {
        let Some(p) = tx.lookup(&t.p)? else { continue };
        for s in candidate_ids(tx, &t.s)? {
            for o in candidate_ids(tx, &t.o)? {
                tx.retract_matching(Some(s), Some(p), Some(o))?;
            }
        }
    }
    Ok(())
}
