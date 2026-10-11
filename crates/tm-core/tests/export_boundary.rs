//! Engine traces for the Statement Graph export boundary paper (Examples 1 and 2).
//!
//! The kernel here is a test-local reference implementation; it is not an engine API.
//! Set `TM_EXPORT_TRACE=<dir>` to write `engine-trace.json` for `validate.py`.
mod common;
use common::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tm_core::*;

type Rows = BTreeMap<u64, Triple>;

fn targets(t: &Triple) -> Vec<u64> {
    [t.s, t.o]
        .into_iter()
        .filter_map(Eid::from_oid)
        .map(Eid::n)
        .collect()
}

/// Least fixpoint of `F(X) = {r : every eid endpoint of r is in X}` by a Kahn-style peel.
fn kernel(rows: &Rows) -> BTreeSet<u64> {
    let mut pending: HashMap<u64, usize> = HashMap::new();
    let mut dependents: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut queue = Vec::new();
    for (&e, t) in rows {
        let ts = targets(t);
        if ts.iter().any(|x| !rows.contains_key(x)) {
            continue;
        }
        for x in &ts {
            dependents.entry(*x).or_default().push(e);
        }
        pending.insert(e, ts.len());
        if ts.is_empty() {
            queue.push(e);
        }
    }
    let mut out = BTreeSet::new();
    while let Some(e) = queue.pop() {
        out.insert(e);
        for d in dependents.get(&e).into_iter().flatten() {
            if let Some(n) = pending.get_mut(d) {
                *n -= 1;
                if *n == 0 {
                    queue.push(*d);
                }
            }
        }
    }
    out
}

fn closed(rows: &Rows) -> bool {
    rows.values()
        .flat_map(targets)
        .all(|x| rows.contains_key(&x))
}

fn by_eid(ts: Vec<Triple>) -> Rows {
    ts.into_iter().map(|t| (t.eid.n(), t)).collect()
}

/// The running example: a route and its distance annotation, asserted at transaction 1.
fn route(db: &mut TestDb) -> (Eid, Eid) {
    let mut ids = None;
    db.tx(|tx| {
        let e1 = tx
            .assert(iri("FRA"), iri("route"), iri("LHR"), Valid::ALWAYS)?
            .eid();
        let e2 = tx
            .assert(e1, iri("distance"), Value::Int(655), Valid::ALWAYS)?
            .eid();
        ids = Some((e1, e2));
        Ok(())
    });
    ids.unwrap()
}

/// Example 1: correcting valid time replays the annotation and leaves live lineage
/// pointing at the retracted route.
fn lineage(db: &mut TestDb) -> (Rows, Rows) {
    let (e1, _) = route(db);
    let patch = Patch {
        v_from: Some(Some(1_000)),
        ..Patch::default()
    };
    db.tx(|tx| tx.supersede(e1, patch).map(|_| ()));
    (by_eid(db.history_all()), by_eid(db.now(None, None, None)))
}

/// Example 2: an object patch naming the retained annotation replays into a two-cycle.
fn correction_cycle(db: &mut TestDb) -> (Rows, Rows) {
    let (e1, e2) = route(db);
    db.tx(|tx| tx.supersede(e1, Patch::object(Value::Stmt(e2))).map(|_| ()));
    (by_eid(db.history_all()), by_eid(db.now(None, None, None)))
}

// @lat: [[tests#Export Boundary#Full History Is Closed]]
host_test! {
    fn full_history_is_closed(db) {
        let (history, now) = lineage(db);
        assert!(closed(&history));
        assert!(!closed(&now));
        assert_eq!(kernel(&history).len(), history.len());
    }
}

// @lat: [[tests#Export Boundary#Snapshot Kernel Drops Dangling Lineage]]
host_test! {
    fn snapshot_kernel_drops_dangling_lineage(db) {
        let (_, now) = lineage(db);
        let supersedes = db.id(&Value::iri(vocab::SYS_SUPERSEDES));
        let kept = kernel(&now);
        for (e, t) in &now {
            assert_eq!(kept.contains(e), t.p != supersedes, "{t:?}");
        }
    }
}

// @lat: [[tests#Export Boundary#History Kernel Drops Only Replay Cycles]]
host_test! {
    fn history_kernel_drops_only_replay_cycles(db) {
        let (history, now) = correction_cycle(db);
        assert!(closed(&history) && !closed(&now));
        let dropped: BTreeSet<u64> =
            history.keys().copied().filter(|e| !kernel(&history).contains(e)).collect();
        // The two replayed copies form the cycle; the lineage row depends on one of them.
        assert_eq!(dropped.len(), 3);
        assert!(dropped.iter().all(|e| history[e].t_ret.is_none()));
        assert_eq!(kernel(&now).len(), now.len() - 3);
    }
}

fn trace_rows(rows: &Rows) -> Vec<String> {
    let end = |o: ObjectId| match Eid::from_oid(o) {
        Some(e) => format!("[\"E\",{}]", e.n()),
        None => format!("[\"T\",{}]", o.raw()),
    };
    rows.values()
        .map(|t| {
            format!(
                "{{\"e\":{},\"s\":{},\"p\":{},\"o\":{},\"a\":{},\"d\":{},\"v1\":{},\"v2\":{}}}",
                t.eid.n(),
                end(t.s),
                end(t.p),
                end(t.o),
                t.t_add.0,
                t.t_ret.map_or("null".into(), |d| d.0.to_string()),
                t.v_from.map_or("null".into(), |v| v.to_string()),
                t.v_to.map_or("null".into(), |v| v.to_string())
            )
        })
        .collect()
}

fn trace_case(name: &str, (history, now): (Rows, Rows)) -> String {
    let list = |s: BTreeSet<u64>| s.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
    format!(
        "\"{name}\":{{\"history\":[{}],\"now_eids\":[{}],\"kernel_history\":[{}],\"kernel_now\":[{}]}}",
        trace_rows(&history).join(","),
        list(now.keys().copied().collect()),
        list(kernel(&history)),
        list(kernel(&now)),
    )
}

#[test]
fn engine_trace() {
    let Ok(dir) = std::env::var("TM_EXPORT_TRACE") else {
        return;
    };
    let a = trace_case("lineage", lineage(&mut TestDb::new(HostKind::Rusqlite)));
    let b = trace_case(
        "correction_cycle",
        correction_cycle(&mut TestDb::new(HostKind::Rusqlite)),
    );
    let path = std::path::Path::new(&dir).join("engine-trace.json");
    std::fs::write(
        path,
        format!(
            "{{\"engine\":\"tm-core {}\",{a},{b}}}\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
}
