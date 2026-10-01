//! Spec `named-graphs` (core): membership statements, graph names, time views,
//! cascade and supersede, and the index coverage of membership lookups.

mod common;
use common::*;

use tm_core::*;

fn g(name: &str) -> Value {
    iri(&format!("g_{name}"))
}

fn add(db: &mut TestDb, name: &str) -> Eid {
    let mut e = None;
    db.tx(|tx| {
        e = Some(
            tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    let _ = name;
    e.unwrap()
}

fn members(db: &mut TestDb, spec: ViewSpec, graph: &Value) -> Vec<Eid> {
    let id = db.id(graph);
    db.read(|e| read::graph_members(e, &spec, id))
}

// @lat: [[tests#Named Graphs#Membership Is Engine Owned]]
host_test! {
    fn membership_is_engine_owned(db) {
        let e = add(db, "x");
        let before = db.count("triple");
        let r = db.try_tx(|tx| {
            tx.assert(e, sys("inGraph"), g("1"), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::ReservedNamespace(m) if m == "urn:tiramemsu:sys:inGraph");
        let r = db.try_tx(|tx| tx.create(e, sys("inGraph"), g("1"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ReservedNamespace(_));
        assert_eq!(db.count("triple"), before);
        // schema statements cannot be members
        let flag = {
            let mut f = None;
            db.tx(|tx| { f = Some(tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?.eid()); Ok(()) });
            f.unwrap()
        };
        let r = db.try_tx(|tx| tx.add_to_graph(flag, g("1"), AssertOpts::default()).map(|_| ()));
        assert_err!(r, Error::ReservedNamespace(_));
    }
}

// @lat: [[tests#Named Graphs#Graph Names Are Validated]]
host_test! {
    fn graph_names_are_validated(db) {
        let e = add(db, "x");
        let before = db.count("triple");
        let r = db.try_tx(|tx| tx.add_to_graph(e, lit("g"), AssertOpts::default()).map(|_| ()));
        assert_err!(r, Error::InvalidGraphName { term } if term.contains("\"g\""));
        let r = db.try_tx(|tx| tx.add_to_graph(e, TxId(1), AssertOpts::default()).map(|_| ()));
        assert_err!(r, Error::InvalidGraphName { .. });
        let r = db.try_tx(|tx| tx.clear_graph(lit("g")).map(|_| ()));
        assert_err!(r, Error::InvalidGraphName { .. });
        let r = db.try_tx(|tx| tx.create_graph(Value::Int(1)).map(|_| ()));
        assert_err!(r, Error::InvalidGraphName { .. });
        assert_eq!(db.count("triple"), before);
        // node and blank-node graphs are fine
        db.tx(|tx| {
            let n = tx.new_node()?;
            let b = tx.new_bnode()?;
            tx.add_to_graph(e, n, AssertOpts::default())?;
            tx.add_to_graph(e, b, AssertOpts::default())?;
            Ok(())
        });
    }
}

// @lat: [[tests#Named Graphs#Error Messages Name The Term]]
#[test]
fn error_messages_name_the_term() {
    let a = Error::InvalidGraphName {
        term: "\"g\"".into(),
    };
    let b = Error::GraphNotFound {
        graph: "<urn:x>".into(),
    };
    let c = Error::GraphExists {
        graph: "<urn:y>".into(),
    };
    assert_eq!(a.to_string(), "invalid graph name: \"g\"");
    assert_eq!(b.to_string(), "graph not found: <urn:x>");
    assert_eq!(c.to_string(), "graph already exists: <urn:y>");
}

// Regression: `graphs` listed a graph once per membership when nothing was declared.
host_test! {
    fn graphs_are_listed_once(db) {
        let mut eids = Vec::new();
        db.tx(|tx| {
            for o in ["b", "c", "d"] {
                eids.push(tx.assert(iri("a"), iri("p"), iri(o), Valid::ALWAYS)?.eid());
            }
            Ok(())
        });
        db.tx(|tx| {
            for e in &eids {
                tx.add_to_graph(*e, g("1"), AssertOpts::default())?;
            }
            Ok(())
        });
        let g1 = db.id(&g("1"));
        assert_eq!(db.read(|x| read::graphs(x, &ViewSpec::NOW)), vec![g1]);
    }
}

// @lat: [[tests#Named Graphs#Statement Can Be In Many Graphs]]
host_test! {
    fn tags_not_containers(db) {
        let e = add(db, "x");
        let mut first = None;
        let mut again = None;
        let mut second = None;
        let rep = db.tx(|tx| {
            first = Some(tx.add_to_graph(e, g("1"), AssertOpts::default())?);
            again = Some(tx.add_to_graph(e, g("1"), AssertOpts::default())?);
            second = Some(tx.add_to_graph(e, g("2"), AssertOpts::default())?);
            Ok(())
        });
        // memberships are reported apart from statements
        assert!(rep.asserted.is_empty());
        assert_eq!(rep.memberships.len(), 2);
        let (m1, new1) = first.unwrap();
        assert!(new1);
        assert_eq!(again.unwrap(), (m1, false));
        assert!(second.unwrap().1);
        assert_eq!(db.now_eids().len(), 3);
        assert_eq!(members(db, ViewSpec::NOW, &g("1")), vec![e]);
        assert_eq!(members(db, ViewSpec::NOW, &g("2")), vec![e]);
        // a later transaction is also idempotent
        let r = db.tx(|tx| { tx.add_to_graph(e, g("1"), AssertOpts::default())?; Ok(()) });
        assert!(r.asserted.is_empty());
        let (g1, g2) = (db.id(&g("1")), db.id(&g("2")));
        let mut want = vec![g1, g2];
        want.sort();
        assert_eq!(db.read(|x| read::graphs(x, &ViewSpec::NOW)), want);
        // a retracted statement cannot be added
        db.tx(|tx| tx.retract(e).map(|_| ()));
        let r = db.try_tx(|tx| tx.add_to_graph(e, g("3"), AssertOpts::default()).map(|_| ()));
        assert_err!(r, Error::NotLive(_));
    }
}

// @lat: [[tests#Named Graphs#Remove And Clear Keep Statements]]
host_test! {
    fn remove_and_clear_keep_statements(db) {
        let e = add(db, "x");
        let mut e2 = None;
        db.tx(|tx| {
            e2 = Some(tx.assert(iri("c"), iri("p"), iri("d"), Valid::ALWAYS)?.eid());
            tx.add_to_graph(e, g("1"), AssertOpts::default())?;
            tx.add_to_graph(e, g("2"), AssertOpts::default())?;
            tx.add_to_graph(e2.unwrap(), g("1"), AssertOpts::default())?;
            Ok(())
        });
        let e2 = e2.unwrap();
        let mut removed = (false, false);
        db.tx(|tx| {
            removed.0 = tx.remove_from_graph(e, g("1"))?;
            removed.1 = tx.remove_from_graph(e, g("9"))?; // never a member: no-op
            Ok(())
        });
        assert_eq!(removed, (true, false));
        let last = db.last_t();
        let retr = db.read(|x| read::triples(x, &ViewSpec::history(), None, None, None));
        assert!(retr.iter().any(|t| t.t_ret == Some(TxId(last))));
        assert!(db.is_live(e));
        assert_eq!(members(db, ViewSpec::NOW, &g("1")), vec![e2]);
        assert_eq!(members(db, ViewSpec::NOW, &g("2")), vec![e]);
        let mut cleared = vec![];
        db.tx(|tx| { cleared = tx.clear_graph(g("1"))?; Ok(()) });
        assert_eq!(cleared.len(), 1);
        assert!(db.is_live(e) && db.is_live(e2));
        assert!(members(db, ViewSpec::NOW, &g("1")).is_empty());
        // create is idempotent; drop retracts the declaration and keeps metadata
        let mut c = vec![];
        db.tx(|tx| {
            c.push(tx.create_graph(g("2"))?.is_new());
            c.push(tx.create_graph(g("2"))?.is_new());
            tx.assert(g("2"), iri("startedBy"), iri("agent7"), Valid::ALWAYS)?;
            Ok(())
        });
        assert_eq!(c, vec![true, false]);
        db.tx(|tx| {
            assert!(tx.graph_declared(g("2"))?);
            Ok(())
        });
        db.tx(|tx| tx.drop_graph(g("2")).map(|_| ()));
        let g2 = db.id(&g("2"));
        assert!(db.read(|x| read::graphs(x, &ViewSpec::NOW)).is_empty());
        assert_eq!(db.now(Some(g2), None, None).len(), 1, "metadata stays");
        assert!(db.is_live(e));
    }
}

// @lat: [[tests#Named Graphs#Views Apply To Membership]]
host_test! {
    fn views_apply_to_membership(db) {
        let e = add(db, "x"); // t1
        let t2 = db.tx(|tx| { tx.add_to_graph(e, g("1"), AssertOpts::default())?; Ok(()) }).t;
        let t3 = db.tx(|tx| { tx.remove_from_graph(e, g("1"))?; Ok(()) }).t;
        let at = |t: TxId| ViewSpec::as_of(TimeRef::Tx(t.0));
        assert_eq!(members(db, at(t2), &g("1")), vec![e]);
        assert!(members(db, at(t3), &g("1")).is_empty());
        assert_eq!(members(db, ViewSpec::history(), &g("1")), vec![e]);
        let g1 = db.id(&g("1"));
        assert_eq!(db.read(|x| read::graphs(x, &at(t2))), vec![g1]);
        assert!(db.read(|x| read::graphs(x, &ViewSpec::NOW)).is_empty());
        // valid time of the membership
        db.tx(|tx| {
            tx.add_to_graph(e, g("v"), AssertOpts {
                valid: Valid::between(day("2025-01-01"), day("2025-07-01")),
                ..Default::default()
            })?;
            Ok(())
        });
        let now = ViewSpec::NOW;
        assert!(members(db, now.valid_at(day("2025-08-01")), &g("v")).is_empty());
        assert_eq!(members(db, now.valid_at(day("2025-03-01")), &g("v")), vec![e]);
        // the member statement must be visible too
        db.tx(|tx| tx.retract(e).map(|_| ()));
        assert!(members(db, ViewSpec::NOW, &g("v")).is_empty());
    }
}

// @lat: [[tests#Named Graphs#Cascade And Supersede Drop Memberships]]
host_test! {
    fn cascade_and_supersede_drop_memberships(db) {
        let e = add(db, "x");
        let mut m = None;
        db.tx(|tx| { m = Some(tx.add_to_graph(e, g("1"), AssertOpts::default())?.0); Ok(()) });
        let m = m.unwrap();
        let rep = db.tx(|tx| tx.retract(e).map(|_| ()));
        assert_eq!(rep.retracted.len(), 1);
        assert_eq!(rep.memberships_retracted, vec![(m, RetKind::Cascade)]);
        let row = db.row(m);
        assert_eq!(row.ret_kind, Some(RetKind::Cascade));
        assert_eq!(row.t_ret, db.row(e).t_ret, "same transaction");
        // supersede does not copy memberships to the replacement
        let e3 = add(db, "y");
        db.tx(|tx| tx.add_to_graph(e3, g("1"), AssertOpts::default()).map(|_| ()));
        let mut new = None;
        db.tx(|tx| {
            new = Some(tx.supersede(e3, Patch { o: Some(iri("z")), ..Default::default() })?);
            Ok(())
        });
        assert!(members(db, ViewSpec::NOW, &g("1")).is_empty());
        assert!(db.is_live(new.unwrap()));
        // cardinality one retracts old memberships with kind cardinality
        db.tx(|tx| {
            tx.assert(iri("age"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
            Ok(())
        });
        let mut old = None;
        db.tx(|tx| {
            let e = tx.assert(iri("alice"), iri("age"), Value::Int(41), Valid::ALWAYS)?.eid();
            old = Some((e, tx.add_to_graph(e, g("1"), AssertOpts::default())?.0));
            Ok(())
        });
        let (oe, om) = old.unwrap();
        let mut ne = None;
        db.tx(|tx| {
            ne = Some(tx.assert(iri("alice"), iri("age"), Value::Int(42), Valid::ALWAYS)?.eid());
            tx.add_to_graph(ne.unwrap(), g("1"), AssertOpts::default())?;
            Ok(())
        });
        assert_eq!(db.row(oe).ret_kind, Some(RetKind::Cardinality));
        assert_eq!(db.row(om).ret_kind, Some(RetKind::Cardinality));
        assert_eq!(members(db, ViewSpec::NOW, &g("1")), vec![ne.unwrap()]);
    }
}

fn plan_of(db: &mut TestDb, sql: &str, params: &[SqlValue]) -> String {
    let sql = format!("EXPLAIN QUERY PLAN {sql}");
    db.quiet(|db| db.read(|e| e.rows(&sql, params)))
        .iter()
        .map(|r| r[3].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join(" | ")
}

// @lat: [[tests#Named Graphs#Membership Lookups Use Live Indexes]]
host_test! {
    /// Task 1.2: both membership lookups are index seeks on the existing indexes.
    /// With no retracted history the (p, o) seek may use `hist_pos` (equal cost);
    /// once memberships have churned and `ANALYZE` ran, it uses the partial `live_pos`.
    fn membership_lookups_use_live_indexes(db) {
        db.tx(|tx| {
            for i in 0..600 {
                let e = tx.assert(iri(&format!("s{i}")), iri("p"), Value::Int(i), Valid::ALWAYS)?.eid();
                tx.add_to_graph(e, g("big"), AssertOpts::default())?;
                if i < 3 {
                    tx.add_to_graph(e, g("small"), AssertOpts::default())?;
                }
            }
            Ok(())
        });
        let (ig, small, big) = (db.id(&sys("inGraph")), db.id(&g("small")), db.id(&g("big")));
        let by_graph = "SELECT a.eid FROM triple m JOIN triple a ON a.eid = m.s \
                        WHERE m.p = ?1 AND m.o = ?2 AND m.t_ret IS NULL AND a.t_ret IS NULL";
        let args = |graph: ObjectId| [SqlValue::Integer(ig.raw()), SqlValue::Integer(graph.raw())];
        for graph in [small, big] {
            let p = plan_of(db, by_graph, &args(graph));
            assert!(p.contains("COVERING INDEX") && p.contains("_pos (p=? AND o=?)"), "{p}");
            assert!(p.contains("INTEGER PRIMARY KEY"), "{p}");
            assert!(!p.contains("SCAN"), "{p}");
        }
        // churn: 500 memberships added to `small` and removed again, then ANALYZE
        db.tx(|tx| {
            for i in 3..500 {
                let e = tx.assert(iri(&format!("s{i}")), iri("p"), Value::Int(i), Valid::ALWAYS)?.eid();
                tx.add_to_graph(e, g("small"), AssertOpts::default())?;
            }
            Ok(())
        });
        db.tx(|tx| {
            tx.clear_graph(g("small"))?;
            for i in 0..3 {
                let e = tx.assert(iri(&format!("s{i}")), iri("p"), Value::Int(i), Valid::ALWAYS)?.eid();
                tx.add_to_graph(e, g("small"), AssertOpts::default())?;
            }
            Ok(())
        });
        db.store().optimize().unwrap();
        let p = plan_of(db, by_graph, &args(small));
        assert!(p.contains("COVERING INDEX live_pos (p=? AND o=?)"), "{p}");
        let pid = db.id(&iri("p"));
        let e = db.now(None, Some(pid), None)[0].eid;
        let of_member = plan_of(
            db,
            "SELECT m.o FROM triple m WHERE m.s = ?1 AND m.p = ?2 AND m.t_ret IS NULL",
            &[SqlValue::Integer(e.oid().raw()), SqlValue::Integer(ig.raw())],
        );
        assert!(of_member.contains("INDEX live_spo"), "{of_member}");
        assert!(!of_member.contains("SCAN"), "{of_member}");
        // stat4 samples exist for the index that serves `GRAPH <g>`
        let n = db.scalar("SELECT count(*) FROM sqlite_stat4 WHERE tbl = 'triple' AND idx = 'live_pos'");
        assert!(n > 0, "sqlite_stat4 has no samples for live_pos");
    }
}

fn stmt_members(db: &mut TestDb, spec: ViewSpec, graph: Eid) -> Vec<Eid> {
    db.read(|x| read::graph_members(x, &spec, graph.oid()))
}

fn put(db: &mut TestDb, s: &str, p: &str, o: &str) -> Eid {
    let mut e = None;
    db.tx(|tx| {
        e = Some(tx.assert(iri(s), iri(p), iri(o), Valid::ALWAYS)?.eid());
        Ok(())
    });
    e.unwrap()
}

// @lat: [[tests#Named Graphs#A Statement Holds A Subgraph]]
host_test! {
    fn a_statement_holds_a_subgraph(db) {
        let edge = put(db, "p7", "enrolledIn", "trial3");
        let inv = put(db, "drSmith", "role", "investigator");
        let site = put(db, "site9", "hosts", "p7");
        db.tx(|tx| {
            assert!(tx.add_to_graph(inv, edge, AssertOpts::default())?.1);
            assert!(tx.add_to_graph(site, edge, AssertOpts::default())?.1);
            // idempotent
            assert!(!tx.add_to_graph(inv, edge, AssertOpts::default())?.1);
            Ok(())
        });
        assert_eq!(stmt_members(db, ViewSpec::NOW, edge), vec![inv, site]);
        assert!(db.read(|x| read::graphs(x, &ViewSpec::NOW)).contains(&edge.oid()));
        // remove and clear work on a statement graph and keep the members
        db.tx(|tx| { assert!(tx.remove_from_graph(site, edge)?); Ok(()) });
        assert_eq!(stmt_members(db, ViewSpec::NOW, edge), vec![inv]);
        db.tx(|tx| { assert_eq!(tx.clear_graph(edge)?.len(), 1); Ok(()) });
        assert!(stmt_members(db, ViewSpec::NOW, edge).is_empty());
        assert!(db.is_live(inv) && db.is_live(site) && db.is_live(edge));
        // a retracted statement cannot gain contents
        db.tx(|tx| tx.retract(edge).map(|_| ()));
        let before = db.count("triple");
        let r = db.try_tx(|tx| tx.add_to_graph(inv, edge, AssertOpts::default()).map(|_| ()));
        assert_err!(r, Error::NotLive(g) if g == edge);
        assert_eq!(db.count("triple"), before);
    }
}

// @lat: [[tests#Named Graphs#Retracting A Statement Graph Keeps Its Members]]
host_test! {
    fn retracting_a_statement_graph_keeps_its_members(db) {
        let edge = put(db, "p7", "enrolledIn", "trial3");
        let inv = put(db, "drSmith", "role", "investigator");
        let mut m = None;
        let t_in = db.tx(|tx| { m = Some(tx.add_to_graph(inv, edge, AssertOpts::default())?.0); Ok(()) }).t;
        let m = m.unwrap();
        let rep = db.tx(|tx| tx.retract(edge).map(|_| ()));
        assert_eq!(rep.memberships_retracted, vec![(m, RetKind::Cascade)]);
        assert_eq!(db.row(m).t_ret, db.row(edge).t_ret, "same transaction");
        assert!(db.is_live(inv));
        assert!(stmt_members(db, ViewSpec::NOW, edge).is_empty());
        let at = ViewSpec::as_of(TimeRef::Tx(t_in.0));
        assert_eq!(stmt_members(db, at, edge), vec![inv]);
    }
}

// @lat: [[tests#Named Graphs#Supersede Carries The Contents Of A Statement Graph]]
host_test! {
    fn supersede_carries_the_contents_of_a_statement_graph(db) {
        let edge = put(db, "p7", "enrolledIn", "trial3");
        let inv = put(db, "drSmith", "role", "investigator");
        db.tx(|tx| {
            tx.add_to_graph(inv, edge, AssertOpts::default())?;
            // the edge is itself a member of an ordinary graph
            tx.add_to_graph(edge, g("1"), AssertOpts::default())?;
            Ok(())
        });
        let mut new = None;
        db.tx(|tx| {
            new = Some(tx.supersede(edge, Patch::object(iri("trial4")))?);
            Ok(())
        });
        let new = new.unwrap();
        // the contents follow the edge, and the member keeps its eid
        assert_eq!(stmt_members(db, ViewSpec::NOW, new), vec![inv]);
        assert!(stmt_members(db, ViewSpec::NOW, edge).is_empty());
        assert!(db.is_live(inv));
        // the edge's own membership in g1 is still dropped
        assert!(members(db, ViewSpec::NOW, &g("1")).is_empty());
        // superseding a member drops its membership in the statement graph
        let mut inv2 = None;
        db.tx(|tx| {
            inv2 = Some(tx.supersede(inv, Patch::object(iri("monitor")))?);
            Ok(())
        });
        assert!(stmt_members(db, ViewSpec::NOW, new).is_empty());
        assert!(db.is_live(inv2.unwrap()));
    }
}
