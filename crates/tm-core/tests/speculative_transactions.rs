//! Spec `speculative-transactions`: `with` on a savepoint, no trace, burned ids.

mod common;
use common::*;

use tm_core::read;
use tm_core::*;

host_test! {
    /// Speculation runs the full engine and exposes uncommitted state.
    fn speculation_exposes_uncommitted_state(db) {
        let (mut e1, mut e2) = (None, None);
        db.tx(|tx| {
            let a = tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS)?.eid();
            e2 = Some(tx.assert(a, iri("note"), lit("n"), Valid::ALWAYS)?.eid());
            e1 = Some(a);
            Ok(())
        });
        let (alice, works) = (db.id(&iri("alice")), db.id(&iri("worksAt")));
        let path = db.path.clone();
        let employers = assert_ok(db.store().speculate(
            |tx| tx.assert(iri("alice"), iri("worksAt"), iri("globex"), Valid::ALWAYS).map(|_| ()),
            |e| {
                let rows = read::triples(e, &ViewSpec::NOW, Some(alice), Some(works), None)?;
                // another connection only sees committed state meanwhile
                let raw = rusqlite::Connection::open(&path).unwrap();
                let n: i64 = raw.query_row("SELECT count(*) FROM triple", [], |r| r.get(0)).unwrap();
                assert_eq!(n, 2);
                let r = TermReader::new(4);
                rows.iter().map(|t| r.decode(e, t.o, false)).collect::<Result<Vec<_>>>()
            },
        ));
        assert!(employers.contains(&iri("globex")));
        // hypothetical retraction with cascade
        let (e1, e2) = (e1.unwrap(), e2.unwrap());
        let live = assert_ok(db.store().speculate(
            |tx| tx.retract(e1).map(|_| ()),
            |e| Ok(read::triples(e, &ViewSpec::NOW, None, None, None)?.iter().map(|t| t.eid).collect::<Vec<_>>()),
        ));
        assert!(!live.contains(&e1) && !live.contains(&e2));
        assert!(db.is_live(e1) && db.is_live(e2));
        // valid-at narrowing inside
        let n = assert_ok(db.store().speculate(
            |tx| tx.assert(iri("bob"), iri("worksAt"), iri("acme"), Valid::between(0, 10)).map(|_| ()),
            |e| Ok(read::triples(e, &ViewSpec::NOW.valid_at(20), None, Some(works), None)?.len()),
        ));
        assert_eq!(n, 1);
    }
}

fn counters(db: &mut TestDb) -> [i64; 4] {
    ["next_stmt", "next_node", "next_bnode", "next_term"].map(|k| db.meta(k))
}

// @lat: [[tests#Storage Invariants#Speculation Leaves No Trace]]
host_test! {
    fn tables_unchanged_after_speculation(db) {
        let mut e1 = None;
        db.tx(|tx| {
            e1 = Some(tx.assert(iri("alice"), iri("name"), lit("Alcie"), Valid::ALWAYS)?.eid());
            tx.set_volatile(iri("alice"), iri("score"), Value::Int(1))?;
            Ok(())
        });
        let before = db.data_snapshot();
        let (t0, c0) = (db.last_t(), counters(db));
        let mut ids = (Vec::new(), None, None);
        assert_ok(db.store().speculate(
            |tx| {
                ids.0.push(tx.assert(iri("x"), iri("text"), lit("a new long string value"), Valid::ALWAYS)?.eid());
                ids.0.push(tx.supersede(e1.unwrap(), Patch::object(lit("Alice")))?);
                ids.1 = Some(tx.new_node()?);
                ids.2 = Some(tx.new_bnode()?);
                tx.set_volatile(iri("alice"), iri("score"), Value::Int(2))?;
                Ok(())
            },
            |_| Ok(()),
        ));
        assert_eq!(db.data_snapshot(), before);
        assert_eq!(db.last_t(), t0);
        assert!(db.events_since(t0).is_empty());
        let c1 = counters(db);
        let max_eid = ids.0.iter().map(|e| e.n() as i64).max().unwrap();
        assert!(c1[0] > max_eid + 1, "{c1:?}");
        assert!(c1[1] > ids.1.unwrap().unsigned_payload() as i64);
        assert!(c1[2] > ids.2.unwrap().unsigned_payload() as i64);
        assert!(c1[3] > c0[3]);
    }
}

host_test! {
    /// Speculation leaves no trace — operations fail, callback fails, numbers.
    fn failures_leave_no_trace(db) {
        db.tx(|tx| {
            tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
            tx.assert(iri("alice"), iri("email"), lit("a@x.org"), Valid::ALWAYS)?;
            Ok(())
        });
        for _ in 0..3 {
            db.tx(|_| Ok(()));
        }
        let before = db.data_snapshot();
        let mut called = false;
        let r: Result<()> = db.store().speculate(
            |tx| tx.assert(iri("bob"), iri("email"), lit("a@x.org"), Valid::ALWAYS).map(|_| ()),
            |_| {
                called = true;
                Ok(())
            },
        );
        assert_err!(r, Error::UniqueViolation { .. });
        assert!(!called);
        assert_eq!(db.data_snapshot(), before);
        let r: Result<()> = db.store().speculate(
            |tx| tx.assert(iri("carol"), iri("p"), iri("q"), Valid::ALWAYS).map(|_| ()),
            |_| Err(Error::custom("callback failed")),
        );
        assert_err!(r, Error::Custom(_));
        assert_eq!(db.data_snapshot(), before);
        assert_eq!(db.last_t(), 4);
        let rep = db.tx(|_| Ok(()));
        assert_eq!(rep.t, TxId(5));
    }
}

host_test! {
    /// Ids allocated during speculation are burned.
    fn ids_are_burned(db) {
        let mut max = 0;
        let mut node = None;
        let mut term = None;
        let iri30 = Value::iri("urn:x:speculative-iri");
        assert_ok(db.store().speculate(
            |tx| {
                for i in 0..20 {
                    max = tx.create(iri("s"), iri("p"), Value::Int(i), Valid::ALWAYS)?.n();
                }
                for _ in 0..7 {
                    node = Some(tx.new_node()?);
                }
                term = Some(tx.encode(&iri30)?);
                Ok(())
            },
            |_| Ok(()),
        ));
        let rep = db.tx(|tx| {
            tx.create(iri("s"), iri("p"), iri("o"), Valid::ALWAYS)?;
            let n = tx.new_node()?;
            assert_ne!(Some(n), node);
            let t = tx.encode(&iri30)?;
            assert!(t.unsigned_payload() > term.unwrap().unsigned_payload());
            Ok(())
        });
        assert!(rep.asserted[0].n() > max);
        // a dry run burns ids the same way; only meta id counters change
        let before = db.data_snapshot();
        let dry = assert_ok(db.try_tx_opts(TxOptions { dry_run: true, ..TxOptions::default() }, |tx| {
            tx.create(iri("d"), iri("p"), iri("o"), Valid::ALWAYS)?;
            tx.create(iri("d"), iri("p"), iri("o2"), Valid::ALWAYS)?;
            Ok(())
        }));
        assert_eq!(db.data_snapshot(), before);
        let rep = db.tx(|tx| tx.create(iri("d"), iri("p"), iri("o"), Valid::ALWAYS).map(|_| ()));
        assert!(dry.asserted.iter().all(|e| rep.asserted[0] > *e));
        // burned ids survive a reopen
        let mut burned = 0;
        assert_ok(db.store().speculate(
            |tx| {
                for _ in 0..5 {
                    burned = tx.create(iri("s"), iri("p"), iri("z"), Valid::ALWAYS)?.n();
                }
                Ok(())
            },
            |_| Ok(()),
        ));
        db.reopen();
        let rep = db.tx(|tx| tx.create(iri("s"), iri("p"), iri("y"), Valid::ALWAYS).map(|_| ()));
        assert!(rep.asserted[0].n() > burned);
        // no counter commit when nothing was allocated
        let c = db.snapshot();
        assert_ok(db.store().speculate(|_| Ok(()), |_| Ok(())));
        assert_eq!(db.snapshot(), c);
    }
}
