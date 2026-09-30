//! The neighbour fetcher's SQL: view predicates only from the view function, the
//! covering indexes of design Decision 5 under every view, virtual-hop shapes.

mod common;

use common::*;
use tiramemsu::*;
use tm_exec::path::automaton::Dir;
use tm_exec::path::fetch::Fetcher;
use tm_exec::path::resolve::{Fetch, RLetter};
use tm_exec::path::row::HopKind;
use tm_exec::scan::{ResolvedTx, ResolvedView};
use tm_rusqlite::RusqliteExec;

fn exec_of(t: &TestDb) -> RusqliteExec {
    let conn = rusqlite::Connection::open(&t.path).unwrap();
    tm_rusqlite::register::register_defaults(&conn).unwrap();
    RusqliteExec::from_connection(conn, Capabilities::default())
}

fn views() -> Vec<(&'static str, ResolvedView)> {
    let now = ResolvedView::NOW;
    let at = |v: ResolvedView| ResolvedView {
        valid: ValidSel::At(1_000),
        ..v
    };
    let asof = ResolvedView {
        tx: ResolvedTx::AsOf(1),
        valid: ValidSel::Unfiltered,
    };
    let hist = ResolvedView {
        tx: ResolvedTx::History,
        valid: ValidSel::Unfiltered,
    };
    vec![
        ("now", now),
        ("asof", asof),
        ("history", hist),
        ("now+valid", at(now)),
        ("asof+valid", at(asof)),
        ("history+valid", at(hist)),
    ]
}

fn letters() -> Vec<(&'static str, RLetter)> {
    let p = ObjectId::from_unsigned(Tag::Iri, 1);
    let mk = |f: Fetch, dir: Dir| RLetter { fetch: f, dir };
    vec![
        ("pred out", mk(Fetch::Pred { p, rv: None }, Dir::Out)),
        ("pred in", mk(Fetch::Pred { p, rv: None }, Dir::In)),
        ("any out", mk(Fetch::Other { excl: vec![] }, Dir::Out)),
        ("any in", mk(Fetch::Other { excl: vec![] }, Dir::In)),
        (
            "subject out",
            mk(Fetch::Virtual(HopKind::Subject), Dir::Out),
        ),
        ("object out", mk(Fetch::Virtual(HopKind::Object), Dir::Out)),
        (
            "predicate out",
            mk(Fetch::Virtual(HopKind::Predicate), Dir::Out),
        ),
        ("subject in", mk(Fetch::Virtual(HopKind::Subject), Dir::In)),
        ("object in", mk(Fetch::Virtual(HopKind::Object), Dir::In)),
        (
            "predicate in",
            mk(Fetch::Virtual(HopKind::Predicate), Dir::In),
        ),
    ]
}

// task 4.1: the `Now` SQL contains the verbatim `t_ret IS NULL`, and no data
#[test]
fn now_sql_uses_the_verbatim_live_predicate() {
    let t = TestDb::new();
    t.tx(|tx| tx.assert(v("a"), v("p"), v("b"), Valid::ALWAYS).map(|_| ()));
    let mut ex = exec_of(&t);
    for (name, l) in letters() {
        let mut f = Fetcher::new(&mut ex, ResolvedView::NOW);
        let sql = f.sql_of(&l).unwrap();
        assert!(sql.contains("t.t_ret IS NULL"), "{name}: {sql}");
        assert!(sql.contains("rarray(?1)"), "{name}: {sql}");
        assert!(!sql.contains('\''), "no string literal in the SQL: {sql}");
    }
    let asof = views()[1].1;
    let mut f = Fetcher::new(&mut ex, asof);
    let sql = f.sql_of(&letters()[0].1).unwrap();
    assert!(sql.contains("t.t_add <= ?"), "{sql}");
}

// task 4.5: EXPLAIN QUERY PLAN of every fetch shape under every view
#[test]
fn every_shape_uses_an_index_or_rowid() {
    let t = TestDb::new();
    t.tx(|tx| {
        for i in 0..400 {
            tx.assert(
                v(&format!("s{}", i % 40)),
                v(&format!("p{}", i % 20)),
                v(&format!("o{}", i / 4)),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    });
    let mut ex = exec_of(&t);
    ex.connection().execute_batch("ANALYZE").unwrap();
    let mut report = Vec::new();
    for (vname, view) in views() {
        for (lname, l) in letters() {
            let sql = {
                let mut f = Fetcher::new(&mut ex, view);
                f.sql_of(&l).unwrap()
            };
            let mut params: Vec<SqlValue> = vec![SqlValue::IntArray(vec![1, 2, 3])];
            let n = sql.matches('?').count();
            let mut st = ex
                .connection()
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap();
            let names = st.parameter_count();
            params.resize(names.max(n), SqlValue::Integer(1));
            let _ = params;
            let plan: Vec<String> = {
                use rusqlite::types::Value as RV;
                use rusqlite::vtab::array::Array;
                let arr: Array = std::rc::Rc::new(vec![RV::Integer(1)]);
                let mut vals: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(arr)];
                for _ in 1..names {
                    vals.push(Box::new(1i64));
                }
                let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
                st.query_map(refs.as_slice(), |r| r.get::<_, String>(3))
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect()
            };
            report.push(format!("{vname} / {lname}: {}", plan.join(" | ")));
        }
    }
    for line in &report {
        let uses_index = line.contains("INDEX") || line.contains("PRIMARY KEY");
        let scans_triple = line.contains("SCAN t") || line.contains("SCAN triple");
        assert!(uses_index && !scans_triple, "{line}");
    }
    insta::assert_snapshot!(report.join("\n"));
}
