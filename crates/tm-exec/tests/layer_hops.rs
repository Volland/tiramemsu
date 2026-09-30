//! Virtual `sys:subject` / `sys:object` / `sys:predicate` hops and paths across
//! statement layers (`layer-hops`).

mod common;

use common::paths::*;
use common::*;
use tiramemsu::*;
use tm_exec::path::row::{virtual_pred_id, virtual_pred_iri, Dir, HopKind};

fn ends_of(
    t: &TestDb,
    view: View<'_>,
    start: ObjectId,
    path: &str,
    mode: PathMode,
) -> Vec<(ObjectId, u32)> {
    let _ = t;
    view.path(start, path, mode, u32::MAX)
        .unwrap()
        .iter()
        .map(|r| (r.end, r.hops))
        .collect()
}

fn set(mut v: Vec<(ObjectId, u32)>) -> Vec<(ObjectId, u32)> {
    v.sort();
    v
}

// layer-hops "Subject hop" / "Object hop" / "Predicate hop" / "Object hop to a literal" /
// "Virtual hop from a plain node" / "No stored sys triples exist"
#[test]
fn forward_virtual_hops() {
    let l = layers();
    let e1 = l.eid("e1").oid();
    let e2 = l.eid("e2").oid();
    let now = l.db().now();
    let id = |n: &str| l.t.id(&v(n));
    let g = |p: &str, from: ObjectId| ends_of(&l.t, now, from, p, REACH);
    assert_eq!(g("sys:subject", e1), vec![(id("alice"), 1)]);
    assert_eq!(g("sys:object", e1), vec![(id("acme"), 1)]);
    assert_eq!(g("sys:predicate", e1), vec![(id("worksAt"), 1)]);
    let lit = now.encode(&Value::Double(0.8)).unwrap().unwrap();
    assert_eq!(g("sys:object", e2), vec![(lit, 1)]);
    assert!(
        g("sys:subject", id("alice")).is_empty(),
        "a plain node has no parts"
    );
    // nothing was ever stored with a `sys:subject` predicate
    let n = l
        .db()
        .read_sql("SELECT count(*) FROM term WHERE lex = 'urn:tiramemsu:sys:subject'")
        .unwrap();
    assert_eq!(n[0][0].as_i64(), Some(0));
}

// "Inverse subject hop finds annotations" / "Inverse object hop finds references" /
// "Inverse subject hop from an entity"
#[test]
fn inverse_virtual_hops() {
    let l = layers();
    let (e1, e2, e3, e7) = (
        l.eid("e1").oid(),
        l.eid("e2").oid(),
        l.eid("e3").oid(),
        l.eid("e7").oid(),
    );
    let now = l.db().now();
    let g = |p: &str, from: ObjectId| set(ends_of(&l.t, now, from, p, REACH));
    assert_eq!(g("^sys:subject", e1), set(vec![(e2, 1), (e3, 1)]));
    assert_eq!(g("^sys:object", e1), vec![(e7, 1)]);
    let alice = l.t.id(&v("alice"));
    let from_alice = g("^sys:subject", alice);
    assert_eq!(from_alice.len(), 2);
    assert!(from_alice.contains(&(e1, 1)));
    // ^sys:predicate: every statement using the predicate
    let works = l.t.id(&v("worksAt"));
    assert_eq!(g("^sys:predicate", works), vec![(e1, 1)]);
}

fn layer_graph() -> (G, Vec<Eid>) {
    // e1 = (alice worksAt acme); e2 = (e1 confidence 0.8); e5 = (e2 source "crawler");
    // e7 = (belief9 supportedBy e1); e8 = (e7 method "llm-extraction")
    let g = G::new();
    let mut e = Vec::new();
    g.t.tx(|tx| {
        let e1 = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        let e2 = tx
            .assert(e1, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?
            .eid();
        let e5 = tx
            .assert(e2, v("source"), s("crawler"), Valid::ALWAYS)?
            .eid();
        let e7 = tx
            .assert(v("belief9"), v("supportedBy"), e1, Valid::ALWAYS)?
            .eid();
        let e8 = tx
            .assert(e7, v("method"), s("llm-extraction"), Valid::ALWAYS)?
            .eid();
        e.extend([e1, e2, e5, e7, e8]);
        Ok(())
    });
    (g, e)
}

// @lat: [[tests#Query#Paths Cross Layers]]
// layer-hops "From a belief through a statement to its entities" / "From an entity to the
// beliefs that rely on its facts" / "Arbitrary layer depth" / "Mixed recursive layer walk"
#[test]
fn paths_cross_layers() {
    let (g, e) = layer_graph();
    let (e1, e2, e5, e7, e8) = (e[0].oid(), e[1].oid(), e[2].oid(), e[3].oid(), e[4].oid());
    let now = g.t.db.now();
    let run = |from: ObjectId, p: &str| set(ends_of(&g.t, now, from, p, REACH));
    assert_eq!(
        run(g.id("belief9"), "supportedBy/(sys:subject|sys:object)"),
        set(vec![(g.id("alice"), 2), (g.id("acme"), 2)])
    );
    assert_eq!(
        run(g.id("alice"), "^sys:subject/^supportedBy"),
        vec![(g.id("belief9"), 2)]
    );
    assert_eq!(run(e1, "(^sys:subject)+"), set(vec![(e2, 1), (e5, 2)]));
    assert_eq!(
        run(e1, "(^sys:subject|^sys:object)+"),
        set(vec![(e2, 1), (e5, 2), (e7, 1), (e8, 2)])
    );
}

// "Retracted statement is not traversed under Now" / "...as of before its retraction" /
// "Inverse hop hides retracted annotations" / "Virtual hop under valid time"
#[test]
fn virtual_hops_respect_the_view() {
    let (g, e) = layer_graph();
    let (e1, e2) = (e[0], e[1]);
    let before = g.t.last_t();
    g.t.tx(|tx| tx.retract(e2).map(|_| ()));
    let now = g.t.db.now();
    let hist = g.t.db.as_of(TimeRef::Tx(before));
    assert_eq!(
        set(ends_of(&g.t, now, e1.oid(), "^sys:subject", REACH)),
        vec![]
    );
    assert_eq!(
        set(ends_of(&g.t, hist, e1.oid(), "^sys:subject", REACH)),
        vec![(e2.oid(), 1)]
    );
    let t_before_retract = g.t.last_t();
    g.t.tx(|tx| tx.retract(e1).map(|_| ()));
    assert!(ends_of(&g.t, g.t.db.now(), e1.oid(), "sys:subject", REACH).is_empty());
    assert_eq!(
        ends_of(
            &g.t,
            g.t.db.as_of(TimeRef::Tx(t_before_retract)),
            e1.oid(),
            "sys:subject",
            REACH
        ),
        vec![(g.id("alice"), 1)]
    );
    // valid time
    let g2 = G::new();
    let mut e = None;
    g2.t.tx(|tx| {
        let from = 1_577_836_800_000; // 2020-01-01
        let to = 1_640_995_200_000; // 2022-01-01
        e = Some(
            tx.assert(
                v("alice"),
                v("worksAt"),
                v("acme"),
                Valid::between(from, to),
            )?
            .eid(),
        );
        Ok(())
    });
    let at = |ms| g2.t.db.now().valid_at(ms);
    assert_eq!(
        ends_of(
            &g2.t,
            at(1_609_459_200_000),
            e.unwrap().oid(),
            "sys:object",
            REACH
        )
        .len(),
        1
    );
    assert!(ends_of(
        &g2.t,
        at(1_672_531_200_000),
        e.unwrap().oid(),
        "sys:object",
        REACH
    )
    .is_empty());
}

// "Virtual hop recorded in the path" / "Stored edge and virtual hop of the same
// statement on one trail" / "Same virtual hop is not reused on a trail"
#[test]
fn virtual_hops_in_path_values_and_trails() {
    let (g, e) = layer_graph();
    let (e1, e7) = (e[0].oid(), e[3].oid());
    let now = g.t.db.now();
    let rows = now
        .path(g.id("belief9"), "supportedBy/sys:subject", TRAIL, 5)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let p = rows[0].path.as_ref().unwrap();
    assert_eq!(p.nodes, vec![g.id("belief9"), e1, g.id("alice")]);
    assert_eq!(
        (p.hops[0].eid, p.hops[0].pred, p.hops[0].dir),
        (e7, g.id("supportedBy"), Dir::Out)
    );
    assert_eq!(
        (p.hops[1].eid, p.hops[1].kind, p.hops[1].dir),
        (e1, HopKind::Subject, Dir::Out)
    );
    assert_eq!(
        virtual_pred_iri(p.hops[1].pred),
        Some(vocab::SYS.to_string().as_str()).map(|_| "urn:tiramemsu:sys:subject")
    );
    // stored edge and virtual hops of one statement are distinct relationships
    let rows = now
        .path(g.id("alice"), "worksAt/^sys:object/sys:subject", TRAIL, 5)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let p = rows[0].path.as_ref().unwrap();
    assert_eq!(
        p.nodes,
        vec![g.id("alice"), g.id("acme"), e1, g.id("alice")]
    );
    let kinds: Vec<_> = p.hops.iter().map(|h| (h.eid, h.kind, h.dir)).collect();
    assert_eq!(
        kinds,
        vec![
            (e1, HopKind::Stored, Dir::Out),
            (e1, HopKind::Object, Dir::In),
            (e1, HopKind::Subject, Dir::Out)
        ]
    );
    assert_eq!(p.hops[1].pred, virtual_pred_id(HopKind::Object));
    // the same virtual hop cannot be reused on a trail (a store with only e1)
    let g1 = G::new();
    let only = g1.edge("alice", "worksAt", "acme").oid();
    let now = g1.t.db.now();
    let rows = now
        .path(only, "(sys:subject|^sys:subject){1,4}", TRAIL, 5)
        .unwrap();
    let ends: Vec<_> = rows.iter().map(|r| (r.end, r.hops)).collect();
    assert_eq!(ends, vec![(g1.id("alice"), 1)]);
}

// "Asserting a virtual hop predicate is rejected"
#[test]
fn virtual_predicates_cannot_be_asserted() {
    let g = G::new();
    g.edge("x", "p", "y");
    let r = g.t.db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("x"),
            Value::iri(tm_exec_vocab::SYS_SUBJECT),
            v("y"),
            Valid::ALWAYS,
        )
        .map(|_| ())
    });
    assert!(matches!(r, Err(Error::ReservedNamespace(_))), "{r:?}");
    assert!(g.now("x", "sys:subject", REACH).is_empty());
}

mod tm_exec_vocab {
    pub const SYS_SUBJECT: &str = "urn:tiramemsu:sys:subject";
}
