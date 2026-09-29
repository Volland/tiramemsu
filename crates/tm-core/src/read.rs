//! Reads: triple-pattern lookup, value resolution and the event log.
//!
//! These functions issue plain SELECTs; the caller decides the snapshot (a read
//! transaction on a reader, or the writer inside a speculation).

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
