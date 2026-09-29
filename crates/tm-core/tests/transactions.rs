//! Spec `transactions`: numbering, instants, metadata, report, options, atomicity.

mod common;
use common::*;

use tm_core::*;

fn dry() -> TxOptions {
    TxOptions {
        dry_run: true,
        ..TxOptions::default()
    }
}

fn tx_numbers(db: &mut TestDb) -> Vec<i64> {
    db.rows("SELECT t FROM tx ORDER BY t")
        .into_iter()
        .map(|r| r[0].as_i64().unwrap())
        .collect()
}

host_test! {
    /// Gap-free transaction numbers — First transaction.
    fn first_transaction(db) {
        let rep = db.tx(|_| Ok(()));
        assert_eq!(rep.t, TxId(1));
        assert_eq!(tx_numbers(db), vec![1]);
    }
}

host_test! {
    /// Gap-free transaction numbers — across failures and speculation.
    fn numbers_stay_gap_free(db) {
        for _ in 0..3 {
            db.tx(|tx| tx.create(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        }
        assert!(db.try_tx(|_| Err(Error::custom("fail"))).is_err());
        assert_ok(db.try_tx_opts(dry(), |tx| tx.create(iri("a"), iri("p"), iri("c"), Valid::ALWAYS).map(|_| ())));
        assert_ok(db.store().speculate(|tx| tx.new_node().map(|_| ()), |_| Ok(())));
        let rep = db.tx(|_| Ok(()));
        assert_eq!(rep.t, TxId(4));
        assert_eq!(tx_numbers(db), vec![1, 2, 3, 4]);
    }
}

fn instants(db: &mut TestDb) -> Vec<i64> {
    db.rows("SELECT instant FROM tx ORDER BY t")
        .into_iter()
        .map(|r| r[0].as_i64().unwrap())
        .collect()
}

// Strictly increasing transaction instants — Clock moves backwards; also checks
// that as_of(Instant) resolves to the right transaction.
// @lat: [[tests#Time Travel#Instants Are Monotonic]]
host_test! {
    fn clock_moves_backwards(db) {
        db.clock.set(10_000);
        let e1 = db.tx(|tx| tx.create(iri("a"), iri("n"), Value::Int(1), Valid::ALWAYS).map(|_| ()));
        db.clock.set(5_000);
        let e2 = db.tx(|tx| tx.create(iri("a"), iri("n"), Value::Int(2), Valid::ALWAYS).map(|_| ()));
        db.clock.set(4_000);
        let e3 = db.tx(|tx| tx.create(iri("a"), iri("n"), Value::Int(3), Valid::ALWAYS).map(|_| ()));
        assert_eq!((e1.instant, e2.instant, e3.instant), (10_000, 10_001, 10_002));
        assert_eq!(instants(db), vec![10_000, 10_001, 10_002]);
        for (ms, t) in [(10_000, 1usize), (10_001, 2), (10_002, 3)] {
            let by_instant = db.triples(ViewSpec::as_of(TimeRef::Instant(ms)), None, None, None);
            let by_t = db.as_of(t as u64);
            assert_eq!(by_instant, by_t);
            assert_eq!(by_instant.len(), t);
        }
        assert!(db.triples(ViewSpec::as_of(TimeRef::Instant(9_999)), None, None, None).is_empty());
    }
}

host_test! {
    /// Strictly increasing transaction instants — Clock stands still / normal clock.
    fn clock_stands_still_and_normal(db) {
        db.clock.set(7_000);
        for _ in 0..3 {
            db.tx(|_| Ok(()));
        }
        assert_eq!(instants(db), vec![7_000, 7_001, 7_002]);
        db.clock.set(20_000);
        let rep = db.tx(|_| Ok(()));
        assert_eq!(rep.instant, 20_000);
    }
}

host_test! {
    /// Every committed transaction is recorded — empty and all-idempotent.
    fn every_commit_is_recorded(db) {
        let rep = db.tx(|_| Ok(()));
        assert_eq!(db.count("tx"), 1);
        assert!(rep.asserted.is_empty() && rep.existing.is_empty());
        assert!(rep.retracted.is_empty() && rep.superseded.is_empty());
        let first = db.tx(|tx| {
            tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?;
            tx.assert(iri("a"), iri("p"), iri("c"), Valid::ALWAYS)?;
            Ok(())
        });
        let rep = db.tx(|tx| {
            tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS)?;
            tx.assert(iri("a"), iri("p"), iri("c"), Valid::between(1, 2))?;
            Ok(())
        });
        assert_eq!(db.count("tx"), 3);
        assert!(rep.asserted.is_empty());
        assert_eq!(rep.existing, first.asserted);
    }
}

host_test! {
    /// Transactions are nodes with metadata triples.
    fn metadata_triples(db) {
        for _ in 0..4 {
            db.tx(|_| Ok(()));
        }
        let rep = db.tx(|tx| {
            tx.meta(sys("author"), iri("agent7"))?;
            tx.meta(sys("reason"), lit("user correction"))?;
            Ok(())
        });
        assert_eq!(rep.t, TxId(5));
        let tx5 = TxId(5).oid();
        let rows = db.now(Some(tx5), None, None);
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].eid, rows[1].eid);
        assert!(rows.iter().all(|r| r.t_add == TxId(5)));
        let author = db.id(&sys("author"));
        let agent = db.id(&iri("agent7"));
        assert!(rows.iter().any(|r| r.p == author && r.o == agent));
        assert_eq!(db.decode(rows.iter().find(|r| r.p != author).unwrap().o), lit("user correction"));
        // metadata of a failed transaction disappears
        let r = db.try_tx(|tx| {
            tx.meta(sys("reason"), lit("doomed"))?;
            Err(Error::custom("fail"))
        });
        assert!(r.is_err());
        assert!(db.history_all().iter().all(|t| t.s != TxId(6).oid()));
        // statements about a past transaction
        for _ in 0..2 {
            db.tx(|_| Ok(()));
        }
        let rep = db.tx(|tx| tx.assert(TxId(5), iri("reviewedBy"), iri("alice"), Valid::ALWAYS).map(|_| ()));
        assert_eq!(rep.t, TxId(8));
        let row = db.row(rep.asserted[0]);
        assert_eq!((row.s, row.t_add), (tx5, TxId(8)));
    }
}

host_test! {
    /// Transaction report — mixed operations and repeated assert.
    fn report_rules(db) {
        let mut b = None;
        let mut c = None;
        let mut d = None;
        db.tx(|tx| {
            b = Some(tx.assert(iri("b"), iri("p"), iri("x"), Valid::ALWAYS)?.eid());
            let ce = tx.assert(iri("c"), iri("p"), iri("x"), Valid::ALWAYS)?.eid();
            d = Some(tx.assert(ce, iri("note"), lit("n"), Valid::ALWAYS)?.eid());
            c = Some(ce);
            Ok(())
        });
        let mut a = None;
        let rep = db.tx(|tx| {
            a = Some(tx.assert(iri("a"), iri("p"), iri("x"), Valid::ALWAYS)?.eid());
            assert_eq!(tx.assert(iri("b"), iri("p"), iri("x"), Valid::ALWAYS)?, Asserted::Existing(b.unwrap()));
            tx.retract(c.unwrap())?;
            Ok(())
        });
        assert_eq!(rep.asserted, vec![a.unwrap()]);
        assert_eq!(rep.existing, vec![b.unwrap()]);
        assert_eq!(
            rep.retracted,
            vec![(c.unwrap(), RetKind::Explicit), (d.unwrap(), RetKind::Cascade)]
        );
        let rep = db.tx(|tx| {
            let first = tx.assert(iri("n"), iri("p"), iri("m"), Valid::ALWAYS)?;
            let second = tx.assert(iri("n"), iri("p"), iri("m"), Valid::ALWAYS)?;
            assert!(first.is_new());
            assert_eq!(second, Asserted::Existing(first.eid()));
            Ok(())
        });
        assert_eq!(rep.asserted.len(), 1);
        assert!(rep.existing.is_empty());
    }
}

fn annotated(db: &mut TestDb, n: usize) -> Eid {
    let mut root = None;
    db.tx(|tx| {
        let r = tx.create(iri("root"), iri("p"), iri("x"), Valid::ALWAYS)?;
        for i in 0..n {
            tx.create(r, iri("note"), Value::Int(i as i64), Valid::ALWAYS)?;
        }
        root = Some(r);
        Ok(())
    });
    root.unwrap()
}

// Transaction options — Defaults (a cascade of 10 000 is allowed, 10 001 fails).
#[test]
fn default_options() {
    let o = TxOptions::default();
    assert!(!o.dry_run);
    assert_eq!(o.max_cascade, 10_000);
    let mut db = TestDb::new(HostKind::Rusqlite);
    let big = annotated(&mut db, 10_000);
    let r = db.try_tx(|tx| tx.retract(big).map(|_| ()));
    assert_err!(r, Error::CascadeLimitExceeded { limit: 10_000, .. });
    let ok = annotated(&mut db, 9_999);
    let rep = db.tx(|tx| tx.retract(ok).map(|_| ()));
    assert_eq!(rep.retracted.len(), 10_000);
}

host_test! {
    /// Transaction options — Dry run returns the would-be report.
    fn dry_run_returns_would_be_report(db) {
        let mut victim = None;
        db.tx(|tx| {
            victim = Some(tx.assert(iri("v"), iri("p"), iri("x"), Valid::ALWAYS)?.eid());
            Ok(())
        });
        let before = db.data_snapshot();
        let rep = assert_ok(db.try_tx_opts(dry(), |tx| {
            tx.assert(iri("a"), iri("p"), iri("x"), Valid::ALWAYS)?;
            tx.assert(iri("b"), iri("p"), iri("x"), Valid::ALWAYS)?;
            tx.retract(victim.unwrap())?;
            Ok(())
        }));
        assert_eq!(rep.asserted.len(), 2);
        assert_eq!(rep.retracted, vec![(victim.unwrap(), RetKind::Explicit)]);
        assert_eq!(rep.t, TxId(2));
        assert_eq!(db.data_snapshot(), before);
        let next = db.tx(|_| Ok(()));
        assert_eq!(next.t, rep.t);
    }
}

host_test! {
    /// Atomic failure leaves no trace — Error after successful operations.
    fn error_after_successful_operations(db) {
        let mut other = None;
        db.tx(|tx| {
            tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
            tx.assert(iri("alice"), iri("email"), lit("a@x.org"), Valid::ALWAYS)?;
            other = Some(tx.assert(iri("o"), iri("p"), iri("q"), Valid::ALWAYS)?.eid());
            tx.set_volatile(iri("alice"), iri("seen"), Value::Int(1))?;
            Ok(())
        });
        let before = db.snapshot();
        let r = db.try_tx(|tx| {
            tx.assert(iri("x"), iri("text"), lit("a brand new long string"), Valid::ALWAYS)?;
            tx.retract(other.unwrap())?;
            tx.set_volatile(iri("alice"), iri("seen"), Value::Int(2))?;
            tx.assert(iri("bob"), iri("email"), lit("a@x.org"), Valid::ALWAYS)?;
            Ok(())
        });
        assert_err!(r, Error::UniqueViolation { .. });
        assert_eq!(db.snapshot(), before);
    }
}

host_test! {
    /// Atomic failure leaves no trace — Caller aborts the body; ids may be reissued.
    fn caller_abort_and_reissued_ids(db) {
        db.tx(|tx| tx.create(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        let before = db.snapshot();
        let mut failed_eid = None;
        let r = db.try_tx(|tx| {
            failed_eid = Some(tx.create(iri("a"), iri("p"), iri("c"), Valid::ALWAYS)?);
            tx.new_node()?;
            Err(Error::custom("caller says no"))
        });
        assert_err!(r, Error::Custom(e) if e.to_string() == "caller says no");
        assert_eq!(db.snapshot(), before);
        let rep = db.tx(|tx| tx.create(iri("a"), iri("p"), iri("d"), Valid::ALWAYS).map(|_| ()));
        assert_eq!(rep.asserted[0], failed_eid.unwrap());
    }
}
