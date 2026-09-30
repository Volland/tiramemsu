//! Reads: triple-pattern lookup, value resolution and the event log.
//!
//! These functions issue plain SELECTs; the caller decides the snapshot (a read
//! transaction on a reader, or the writer inside a speculation).

use std::collections::HashSet;

use crate::error::Result;
use crate::event::{Event, Op};
use crate::exec::{Executor, Params, SqlValue};
use crate::id::{Eid, ObjectId, TxId};
use crate::report::{RetKind, Triple};
use crate::view::{scan_predicates, TxSel, ViewSpec};

fn row_to_triple(r: &[SqlValue], mask_retraction: bool) -> Triple {
    let i = |k: usize| r[k].as_i64();
    Triple {
        eid: Eid::from_oid(ObjectId::from_raw(i(0).unwrap_or(0))).unwrap_or(Eid::new(0)),
        s: ObjectId::from_raw(i(1).unwrap_or(0)),
        p: ObjectId::from_raw(i(2).unwrap_or(0)),
        o: ObjectId::from_raw(i(3).unwrap_or(0)),
        t_add: TxId(i(4).unwrap_or(0) as u64),
        t_ret: if mask_retraction {
            None
        } else {
            i(5).map(|t| TxId(t as u64))
        },
        v_from: i(6),
        v_to: i(7),
        ret_kind: if mask_retraction {
            None
        } else {
            i(8).and_then(RetKind::from_i64)
        },
    }
}

/// Every statement selected by `spec` that matches the bound positions, ordered by
/// eid. As-of rows report `t_ret` and `ret_kind` as `None`.
///
/// `None` in a position is a wildcard. Call it inside [`Store::read`](crate::Store::read)
/// so the lookup runs in one read transaction.
///
/// # Example
///
/// ```
/// use tm_core::{read, vocab::v, Store, StoreOptions, TimeRef, TxOptions, Valid, Value, ViewSpec};
/// use tm_rusqlite as host;
///
/// # let dir = tempfile::tempdir().unwrap();
/// # let path = dir.path().join("db");
/// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
/// let t1 = store.transact(TxOptions::default(), |tx| {
///     tx.assert(Value::iri(v("a")), Value::iri(v("p")), Value::Int(1), Valid::ALWAYS)?;
///     Ok(())
/// })?.t;
/// store.transact(TxOptions::default(), |tx| {
///     let eid = tx.assert(Value::iri(v("a")), Value::iri(v("p")), Value::Int(1), Valid::ALWAYS)?.eid();
///     tx.retract(eid)?;
///     Ok(())
/// })?;
/// let then = store.read(|e| read::triples(e, &ViewSpec::as_of(TimeRef::Tx(t1.0)), None, None, None))?;
/// let now = store.read(|e| read::triples(e, &ViewSpec::now(), None, None, None))?;
/// assert_eq!((then.len(), now.len()), (1, 0));
/// # Ok::<(), tm_core::Error>(())
/// ```
pub fn triples(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    s: Option<ObjectId>,
    p: Option<ObjectId>,
    o: Option<ObjectId>,
) -> Result<Vec<Triple>> {
    let mut params = Params::new();
    let mut conds = Vec::new();
    for (col, v) in [("s", s), ("p", p), ("o", o)] {
        if let Some(v) = v {
            let ph = params.push(v.raw());
            conds.push(format!("a.{col} = {ph}"));
        }
    }
    let time = scan_predicates(spec, "a", &mut params);
    if !time.is_empty() {
        conds.push(time);
    }
    let where_ = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    let sql = format!(
        "SELECT a.eid, a.s, a.p, a.o, a.t_add, a.t_ret, a.v_from, a.v_to, a.ret_kind \
         FROM triple a{where_} ORDER BY a.eid"
    );
    let mask = matches!(spec.tx, TxSel::AsOf(_));
    let mut out = Vec::new();
    exec.query(&sql, params.values(), &mut |r| {
        out.push(row_to_triple(r, mask));
        Ok(())
    })?;
    Ok(out)
}

/// The values of `(s, key)`: objects of the statements selected by the view, or,
/// only for a now view and only when there is no such statement, the volatile value.
// @lat: [[storage#Volatile Table]]
pub fn values(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    s: ObjectId,
    key: ObjectId,
) -> Result<Vec<ObjectId>> {
    let objs: Vec<ObjectId> = triples(exec, spec, Some(s), Some(key), None)?
        .into_iter()
        .map(|t| t.o)
        .collect();
    if !objs.is_empty() || spec.tx != TxSel::Now {
        return Ok(objs);
    }
    Ok(exec
        .query_i64(
            "SELECT value FROM volatile WHERE s = ?1 AND key = ?2",
            &[SqlValue::Integer(s.raw()), SqlValue::Integer(key.raw())],
        )?
        .map(ObjectId::from_raw)
        .into_iter()
        .collect())
}

/// Every event with `t > since`, ordered by time, asserts before retracts, then eid.
// @lat: [[time-model#Event Log]]
pub fn events_since(exec: &mut dyn Executor, since: u64) -> Result<Vec<Event>> {
    let mut out = Vec::new();
    exec.query(
        "SELECT t, eid, op, kind FROM event WHERE t > ?1 ORDER BY t, op, eid",
        &[SqlValue::Integer(since as i64)],
        &mut |r| {
            out.push(Event {
                t: TxId(r[0].as_i64().unwrap_or(0) as u64),
                eid: Eid::from_oid(ObjectId::from_raw(r[1].as_i64().unwrap_or(0)))
                    .unwrap_or(Eid::new(0)),
                op: if r[2].as_str() == Some("retract") {
                    Op::Retract
                } else {
                    Op::Assert
                },
                kind: r[3].as_i64().and_then(RetKind::from_i64),
            });
            Ok(())
        },
    )?;
    Ok(out)
}

/// The last committed transaction number.
pub fn last_t(exec: &mut dyn Executor) -> Result<u64> {
    Ok(exec
        .query_i64("SELECT value FROM meta WHERE key = 'last_t'", &[])?
        .unwrap_or(0) as u64)
}

/// The instant of transaction `t`, if it exists.
pub fn tx_instant(exec: &mut dyn Executor, t: u64) -> Result<Option<i64>> {
    exec.query_i64(
        "SELECT instant FROM tx WHERE t = ?1",
        &[SqlValue::Integer(t as i64)],
    )
}

/// True when statement `eid` is selected by `spec`.
pub(crate) fn visible(exec: &mut dyn Executor, spec: &ViewSpec, eid: Eid) -> Result<bool> {
    let mut params = Params::new();
    let e = params.push(eid.oid().raw());
    let mut conds = vec![format!("a.eid = {e}")];
    let time = scan_predicates(spec, "a", &mut params);
    if !time.is_empty() {
        conds.push(time);
    }
    let sql = format!("SELECT a.eid FROM triple a WHERE {}", conds.join(" AND "));
    Ok(exec.query_i64(&sql, params.values())?.is_some())
}

/// The statements that stand on `root` in `spec`: `root` first, then, breadth-first,
/// every statement selected by the view whose subject or object is a statement
/// already reached. Each expansion is appended in ascending eid order and a visited
/// set makes reference cycles terminate, so under the now view this is the cascade
/// set a retraction of `root` would retract, in the same order. Empty when `root` is
/// not selected by the view.
///
/// Use it to see what a retraction would take with it without the writer, or, with
/// an as-of or history view, what depended on a statement then. The read is never
/// truncated: the cascade limit `max_cascade` applies to retractions only.
///
/// # Errors
///
/// [`crate::Error::Sqlite`] on a read failure.
///
/// # Example
///
/// ```
/// use tm_core::{read, vocab::v, Store, StoreOptions, TimeRef, TxOptions, Valid, Value, ViewSpec};
/// use tm_rusqlite as host;
///
/// # let dir = tempfile::tempdir().unwrap();
/// # let path = dir.path().join("db");
/// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
/// let r = store.transact(TxOptions::default(), |tx| {
///     let fact = tx.assert(Value::iri(v("alice")), Value::iri(v("worksAt")), Value::iri(v("acme")), Valid::ALWAYS)?.eid();
///     tx.assert(fact, Value::iri(v("confidence")), Value::Double(0.8), Valid::ALWAYS)?;
///     Ok(())
/// })?;
/// let (fact, layer) = (r.asserted[0], r.asserted[1]);
/// store.transact(TxOptions::default(), |tx| tx.retract(fact).map(|_| ()))?;
///
/// let now = store.read(|e| read::dependents(e, &ViewSpec::now(), fact))?;
/// let then = store.read(|e| read::dependents(e, &ViewSpec::as_of(TimeRef::Tx(r.t.0)), fact))?;
/// assert!(now.is_empty());
/// assert_eq!(then, vec![fact, layer]);
/// # Ok::<(), tm_core::Error>(())
/// ```
// @lat: [[time-model#Cascade#Dependents]]
pub fn dependents(exec: &mut dyn Executor, spec: &ViewSpec, root: Eid) -> Result<Vec<Eid>> {
    if !visible(exec, spec, root)? {
        return Ok(Vec::new());
    }
    // one statement per expansion, the shape of `Tx::cascade_set` with the view's
    // time predicates in place of `t_ret IS NULL`
    let mut params = Params::new();
    let e = params.push(0i64);
    let mut halves = Vec::with_capacity(2);
    for col in ["s", "o"] {
        let mut conds = vec![format!("a.{col} = {e}")];
        let time = scan_predicates(spec, "a", &mut params);
        if !time.is_empty() {
            conds.push(time);
        }
        halves.push(format!(
            "SELECT a.eid FROM triple a WHERE {}",
            conds.join(" AND ")
        ));
    }
    let sql = format!("{} ORDER BY eid", halves.join(" UNION "));
    let mut values = params.values().to_vec();
    let mut order = vec![root];
    let mut seen: HashSet<Eid> = HashSet::from([root]);
    let mut i = 0;
    while i < order.len() {
        values[0] = SqlValue::Integer(order[i].oid().raw());
        i += 1;
        let mut found = Vec::new();
        exec.query(&sql, &values, &mut |r| {
            if let Some(x) = r[0]
                .as_i64()
                .and_then(|r| Eid::from_oid(ObjectId::from_raw(r)))
            {
                found.push(x);
            }
            Ok(())
        })?;
        for x in found {
            if seen.insert(x) {
                order.push(x);
            }
        }
    }
    Ok(order)
}

/// The [`Bundle`](crate::Bundle) of `root` in `spec`: the statements that stand on
/// `root` ([`dependents`]) plus every visible statement they reference in subject or
/// object position, transitively, so that no imported statement dangles.
///
/// Left out, with everything that references them: statements whose subject or
/// object is a transaction (transaction numbers are local to a file), statements
/// whose predicate is engine bookkeeping a user write cannot repeat
/// (`sys:supersedes`, `sys:confirmedBy`; `sys:inGraph` memberships are kept), and
/// statements that reference a statement not visible in the view. Statements come
/// references first (ties by eid), local ids are positions, and `NODE` / `BNODE` ids
/// become bundle-local anonymous labels, so the same view of the same data always
/// gives an equal bundle.
///
/// # Errors
///
/// [`crate::Error::NotLive`] when `root` is not visible in the view, and
/// [`crate::Error::Unsupported`] when `root` itself is one of the statements left out.
///
/// # Example
///
/// ```
/// use tm_core::{read, vocab::v, BTerm, Store, StoreOptions, TxOptions, Valid, Value, ViewSpec};
/// use tm_rusqlite as host;
///
/// # let dir = tempfile::tempdir().unwrap();
/// # let path = dir.path().join("db");
/// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
/// let r = store.transact(TxOptions::default(), |tx| {
///     let job = tx.assert(Value::iri(v("alice")), Value::iri(v("worksAt")), Value::iri(v("acme")), Valid::ALWAYS)?.eid();
///     tx.assert(Value::iri(v("belief9")), Value::iri(v("supportedBy")), job, Valid::ALWAYS)?;
///     Ok(())
/// })?;
/// // the belief brings the fact it relies on, and the fact comes first
/// let b = store.read(|e| read::bundle(e, &ViewSpec::now(), r.asserted[1]))?;
/// assert_eq!(b.statements.len(), 2);
/// assert_eq!(b.root, 1);
/// assert_eq!(b.statements[1].o, BTerm::Stmt(0));
/// # Ok::<(), tm_core::Error>(())
/// ```
pub fn bundle(exec: &mut dyn Executor, spec: &ViewSpec, root: Eid) -> Result<crate::Bundle> {
    crate::bundle::export(exec, spec, root)
}

/// The ids of `sys:inGraph`, `rdf:type` and `sys:Graph`; `None` for one that is not
/// interned yet (a store that never used graphs has none of them).
fn graph_terms(exec: &mut dyn Executor) -> Result<[Option<ObjectId>; 3]> {
    use crate::term::TermReader;
    use crate::value::Value;
    use crate::vocab;
    let mut out = [None; 3];
    for (slot, iri) in out
        .iter_mut()
        .zip([vocab::SYS_IN_GRAPH, vocab::RDF_TYPE, vocab::SYS_GRAPH])
    {
        *slot = TermReader::encode(exec, &Value::iri(iri))?;
    }
    Ok(out)
}

/// The eids of the statements that are members of `graph` in `spec`: the member
/// statement and its `sys:inGraph` membership are both visible in the view.
// @lat: [[data-model#Named Graphs]]
pub fn graph_members(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    graph: ObjectId,
) -> Result<Vec<Eid>> {
    let [Some(ig), _, _] = graph_terms(exec)? else {
        return Ok(Vec::new());
    };
    let mut params = Params::new();
    let (p, o) = (params.push(ig.raw()), params.push(graph.raw()));
    let mut conds = vec![format!("m.p = {p}"), format!("m.o = {o}")];
    for alias in ["m", "a"] {
        let t = scan_predicates(spec, alias, &mut params);
        if !t.is_empty() {
            conds.push(t);
        }
    }
    let sql = format!(
        "SELECT DISTINCT a.eid FROM triple m JOIN triple a ON a.eid = m.s WHERE {} ORDER BY a.eid",
        conds.join(" AND ")
    );
    let mut out = Vec::new();
    exec.query(&sql, params.values(), &mut |r| {
        if let Some(e) = r[0]
            .as_i64()
            .and_then(|r| Eid::from_oid(ObjectId::from_raw(r)))
        {
            out.push(e);
        }
        Ok(())
    })?;
    Ok(out)
}

/// The graphs of `spec`: every graph with a visible membership of a visible
/// statement, plus every declared `(g rdf:type sys:Graph)`, ascending.
pub fn graphs(exec: &mut dyn Executor, spec: &ViewSpec) -> Result<Vec<ObjectId>> {
    let [ig, ty, sg] = graph_terms(exec)?;
    let mut params = Params::new();
    let mut parts = Vec::new();
    if let Some(ig) = ig {
        let p = params.push(ig.raw());
        let mut conds = vec![format!("m.p = {p}")];
        for alias in ["m", "a"] {
            let t = scan_predicates(spec, alias, &mut params);
            if !t.is_empty() {
                conds.push(t);
            }
        }
        parts.push(format!(
            // DISTINCT: without a UNION, a graph would be listed once per membership.
            "SELECT DISTINCT m.o AS g FROM triple m JOIN triple a ON a.eid = m.s WHERE {}",
            conds.join(" AND ")
        ));
    }
    if let (Some(ty), Some(sg)) = (ty, sg) {
        let (p, o) = (params.push(ty.raw()), params.push(sg.raw()));
        let mut conds = vec![format!("d.p = {p}"), format!("d.o = {o}")];
        let t = scan_predicates(spec, "d", &mut params);
        if !t.is_empty() {
            conds.push(t);
        }
        parts.push(format!(
            "SELECT d.s AS g FROM triple d WHERE {}",
            conds.join(" AND ")
        ));
    }
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!("{} ORDER BY g", parts.join(" UNION "));
    let mut out = Vec::new();
    exec.query(&sql, params.values(), &mut |r| {
        out.push(ObjectId::from_raw(r[0].as_i64().unwrap_or(0)));
        Ok(())
    })?;
    Ok(out)
}
