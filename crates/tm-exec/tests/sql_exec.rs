//! Spec `sql-execution`: pattern and operator compilation, isomorphism, set versus
//! bag, missing values, filter truth values, value comparison, ordering and
//! aggregates; plus the filter push-down of design D4.

mod common;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::*;
use tiramemsu::ir::{Op, View};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn bag() -> Semantics {
    Semantics::sparql().with_graph_set(GraphSet::BagOfEids)
}

fn dt(lex: &str) -> Value {
    Value::literal(lex, Some(tm_core::vocab::XSD_DATETIME), None)
}

fn people() -> TestDb {
    let t = TestDb::new();
    t.tx(|tx| {
        for (p, n, a) in [
            ("alice", "Alice", Some(30)),
            ("bob", "Bob", Some(25)),
            ("carol", "Carol", None),
        ] {
            tx.assert(v(p), v("name"), s(n), Valid::ALWAYS)?;
            if let Some(a) = a {
                tx.assert(v(p), v("age"), Value::Int(a), Valid::ALWAYS)?;
            }
        }
        tx.assert(v("alice"), v("email"), s("a@x.org"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("email"), s("b@x.org"), Valid::ALWAYS)?;
        tx.assert(v("q1"), v("contact"), s("a@x.org"), Valid::ALWAYS)?;
        tx.assert(v("q2"), v("contact"), s("b@x.org"), Valid::ALWAYS)?;
        for p in ["alice", "bob", "carol"] {
            tx.assert(v(p), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        }
        Ok(())
    });
    t
}

// sql-execution "Data never appears in SQL text" / "Stable text across constant values"
#[test]
fn no_data_in_sql_text() {
    let t = TestDb::new();
    let evil = "x' OR 1=1 --";
    t.tx(|tx| {
        tx.assert(v("alice"), v("says"), s(evil), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("says"), s("hello"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("says"), s("hi"), Valid::ALWAYS)?;
        Ok(())
    });
    t.advance_to(50);
    let q = |who: &str, t: u64| {
        let asof = b().at(View::as_of_tx(t));
        b().query(Op::join(vec![
            b().triple(who, "v:says", s(evil)),
            asof.triple(who, "v:says", "?o"),
        ]))
    };
    let ex = explain(&t.db.now(), &q("v:alice", 42));
    let sql = ex.sql.clone().unwrap();
    let alice = t.id(&v("alice")).raw().to_string();
    for bad in [
        "x' OR 1=1",
        "v:alice",
        "urn:tiramemsu",
        "42",
        alice.as_str(),
    ] {
        assert!(!sql.contains(bad), "{bad} in {sql}");
    }
    let r = run(&t.db.now(), &b().query(b().triple("?s", "v:says", s(evil))));
    assert_eq!(rows(&r), expect(&[&["alice"]]));
    // same shape, other constants and time values: byte-identical text
    let other = explain(&t.db.now(), &q("v:bob", 7)).sql.unwrap();
    assert_eq!(sql, other);
    assert_ne!(ex.params, explain(&t.db.now(), &q("v:bob", 7)).params);
}

// sql-execution "Repeated variable in one pattern" / "Variable predicate" /
// "Shared variable across patterns"
#[test]
fn triple_pattern_compilation() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("a"), v("knows"), v("a"), Valid::ALWAYS)?;
        tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("acme"), v("locatedIn"), v("paris"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("name"), s("Alice"), Valid::ALWAYS)?;
        Ok(())
    });
    let r = run(&t.db.now(), &b().query(b().triple("?x", "v:knows", "?x")));
    assert_eq!(rows(&r), expect(&[&["a"]]));
    let q = IrQuery::new(b().triple("v:alice", "?p", "?o"), bag());
    let r = run(&t.db.now(), &q);
    assert_eq!(
        sorted(&r),
        expect(&[&["name", "Alice"], &["worksAt", "acme"]])
    );
    let q = b().query(b().bgp(&[("?a", "v:worksAt", "?c"), ("?c", "v:locatedIn", "?city")]));
    assert_eq!(
        rows(&run(&t.db.now(), &q)),
        expect(&[&["alice", "acme", "paris"]])
    );
}

// sql-execution "Optional condition inside the join"
#[test]
fn optional_condition_inside_join() {
    let t = people();
    let q = b().query(Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "v:age", "?a"),
        Some(Expr::gt(Expr::var("a"), Expr::val(Value::Int(30)))),
    ));
    let r = run(&t.db.now(), &q);
    assert_eq!(
        sorted(&r),
        expect(&[
            &["alice", "Alice", "-"],
            &["bob", "Bob", "-"],
            &["carol", "Carol", "-"]
        ])
    );
    let q = b().query(Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "v:age", "?a"),
        Some(Expr::gt(Expr::var("a"), Expr::val(Value::Int(26)))),
    ));
    assert_eq!(run(&t.db.now(), &q).get(0, "a"), Some(&Value::Int(30)));
}

// sql-execution "Union keeps duplicates" / "Distinct projection"
#[test]
fn union_and_distinct() {
    let t = people();
    let alice = || b().triple("?x", "v:name", s("Alice"));
    let r = run(&t.db.now(), &b().query(Op::union(vec![alice(), alice()])));
    assert_eq!(rows(&r), expect(&[&["alice"], &["alice"]]));
    let q = b().query(b().triple("?a", "v:worksAt", "?c").project_distinct(&["c"]));
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["acme"]]));
}

fn five() -> TestDb {
    let t = TestDb::new();
    t.tx(|tx| {
        for (i, n) in ["e", "a", "d", "b", "c"].iter().enumerate() {
            tx.assert(v(&format!("p{i}")), v("n"), s(n), Valid::ALWAYS)?;
        }
        Ok(())
    });
    t
}

// sql-execution "Skip without limit" / "Limit zero"
#[test]
fn skip_and_limit() {
    let t = five();
    let q = b().query(
        b().triple("?p", "v:n", "?n")
            .order_limit(vec![Key::asc(Expr::var("n"))], Some(2), None)
            .project(&["n"]),
    );
    assert_eq!(
        rows(&run(&t.db.now(), &q)),
        expect(&[&["c"], &["d"], &["e"]])
    );
    let sql = explain(&t.db.now(), &q).sql.unwrap();
    assert!(sql.contains("LIMIT -1 OFFSET ?"), "{sql}");
    let q = b().query(
        b().triple("?p", "v:n", "?n")
            .order_limit(vec![], None, Some(0)),
    );
    let r = run(&t.db.now(), &q);
    assert!(r.is_empty());
    assert_eq!(cols(&r), ["p", "n"]);
}

fn knows_group(b: &IrBuilder) -> Op {
    Op::join(vec![
        Op::Triple(b.t("?x", "v:knows", "?y").with_eid("?r1").in_group(1)),
        Op::Triple(b.t("?y", "v:knows", "?z").with_eid("?r2").in_group(1)),
    ])
}

// sql-execution "A self-loop cannot be traversed twice" / "Same pattern under
// homomorphism" / "Parallel self-loops" / "Different match groups are not constrained"
// @lat: [[tests#Query#Isomorphism Excludes Reused Eids]]
#[test]
fn relationship_isomorphism() {
    let t = TestDb::new();
    let mut eids = Vec::new();
    t.tx(|tx| {
        eids.push(tx.create(v("a"), v("knows"), v("a"), Valid::ALWAYS)?);
        Ok(())
    });
    let c = IrBuilder::cypher();
    let iso = c.query(knows_group(&c));
    assert!(run(&t.db.now(), &iso).is_empty());
    let homo = IrQuery::new(
        knows_group(&c),
        Semantics::cypher().with_match_mode(MatchMode::Homomorphism),
    );
    let e1 = eids[0].to_string();
    assert_eq!(
        rows(&run(&t.db.now(), &homo)),
        expect(&[&["a", "a", &e1, "a", &e1]])
    );
    t.tx(|tx| {
        eids.push(tx.create(v("a"), v("knows"), v("a"), Valid::ALWAYS)?);
        Ok(())
    });
    let e2 = eids[1].to_string();
    let r = run(&t.db.now(), &iso);
    assert_eq!(
        sorted(&r),
        expect(&[&["a", "a", &e1, "a", &e2], &["a", "a", &e2, "a", &e1]])
    );
    assert_eq!(run(&t.db.now(), &homo).len(), 4);
    // different match groups are not constrained
    let groups = c.query(Op::join(vec![
        Op::Triple(c.t("?x", "v:knows", "?y").with_eid("?r1").in_group(1)),
        Op::Triple(c.t("?y", "v:knows", "?z").with_eid("?r2").in_group(2)),
    ]));
    assert_eq!(run(&t.db.now(), &groups).len(), 4);
    // provably distinct predicates add no constraint
    let sql = explain(
        &t.db.now(),
        &c.query(Op::join(vec![
            Op::Triple(c.t("?x", "v:knows", "?y").with_eid("?r1").in_group(1)),
            Op::Triple(c.t("?y", "v:name", "?z").with_eid("?r2").in_group(1)),
        ])),
    );
    assert!(sql.short_circuit || !sql.sql.unwrap().contains("<>"));
    let sql = explain(&t.db.now(), &iso).sql.unwrap();
    assert!(sql.contains("t0.eid <> t1.eid"), "{sql}");
}

// sql-execution "Parallel edges" / "Count under set semantics"
// @lat: [[tests#Query#Set Semantics Dedupes Eids]]
#[test]
fn set_versus_bag_parallel_edges() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        Ok(())
    });
    let op = || b().triple("v:alice", "v:called", "?x");
    assert_eq!(run(&t.db.now(), &b().query(op())).len(), 1);
    assert_eq!(run(&t.db.now(), &IrQuery::new(op(), bag())).len(), 2);
    let count = b().query(op().aggregate(&[], vec![Agg::count_star("n")]));
    assert_eq!(rows(&run(&t.db.now(), &count)), expect(&[&["1"]]));
    // binding the eid gives one row per statement under both settings
    let eid = b().query(Op::Triple(
        b().t("v:alice", "v:called", "?x").with_eid("?r"),
    ));
    assert_eq!(run(&t.db.now(), &eid).len(), 2);
    // Project{distinct} over a BGP elides the canonical-eid predicate
    let d = b().query(op().project_distinct(&["x"]));
    assert!(!explain(&t.db.now(), &d)
        .sql
        .unwrap()
        .contains("min(x0.eid)"));
    assert_eq!(run(&t.db.now(), &d).len(), 1);
}

// sql-execution "Episodes under set semantics"
#[test]
fn set_semantics_episodes() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(0, 1_000),
        )?;
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(2_000, 3_000),
        )?;
        Ok(())
    });
    let q = b().query(b().triple("v:alice", "v:worksAt", "?c"));
    assert_eq!(run(&t.db.now(), &q).len(), 1);
    assert_eq!(
        run(&t.db.now(), &IrQuery::new(q.root.clone(), bag())).len(),
        2
    );
}

// sql-execution "History under set semantics" and "Re-assert after retract under History"
#[test]
fn set_semantics_history() {
    let t = TestDb::new();
    let mut e = None;
    t.tx(|tx| {
        e = Some(
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    t.tx(|tx| tx.retract(e.unwrap()).map(|_| ()));
    t.tx(|tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)
            .map(|_| ())
    });
    let h = b().at(View::history());
    let q = h.query(h.triple("v:alice", "v:worksAt", "?c"));
    assert_eq!(run(&t.db.now(), &q).len(), 1);
    let q = h.query(Op::Triple(h.t("v:alice", "v:worksAt", "?c").with_eid("?r")));
    assert_eq!(run(&t.db.now(), &q).len(), 2);
}

// sql-execution "Predicate without duplicates skips removal" / "A parallel edge turns
// removal on" (decision D27)
#[test]
fn duplicate_removal_only_for_multi_eid_predicates() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("name"), s("Alice"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        Ok(())
    });
    let names = b().query(b().triple("?x", "v:name", "?n"));
    assert!(!explain(&t.db.now(), &names).sql.unwrap().contains("min(x"));
    let called = b().query(b().triple("v:alice", "v:called", "?x"));
    let before = explain(&t.db.now(), &called).sql.unwrap();
    assert!(!before.contains("min(x"), "{before}");
    assert_eq!(run(&t.db.now(), &called).len(), 1);
    t.tx(|tx| {
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)
            .map(|_| ())
    });
    let multi = t.db.read_sql("SELECT count(*) FROM pred_multi").unwrap();
    assert_eq!(multi[0][0], SqlValue::Integer(1));
    let after = explain(&t.db.now(), &called).sql.unwrap();
    assert!(after.contains("min(x0.eid)"), "{after}");
    assert_eq!(run(&t.db.now(), &called).len(), 1);
    // a variable predicate always keeps it
    let var = b().query(b().triple("v:alice", "?p", "?x"));
    assert!(explain(&t.db.now(), &var)
        .sql
        .unwrap()
        .contains("min(x0.eid)"));
}

fn after_optional() -> Op {
    Op::join(vec![
        Op::left_join(
            b().triple("?p", "v:name", "?n"),
            b().triple("?p", "v:email", "?e"),
            None,
        ),
        b().triple("?q", "v:contact", "?e"),
    ])
}

// sql-execution "Join after optional, unbound semantics" / "…, null semantics"
#[test]
fn join_after_optional() {
    let t = people();
    let r = run(
        &t.db.now(),
        &b().query(after_optional().project(&["p", "q", "e"])),
    );
    assert_eq!(
        sorted(&r),
        expect(&[
            &["alice", "q1", "a@x.org"],
            &["bob", "q2", "b@x.org"],
            &["carol", "q1", "a@x.org"],
            &["carol", "q2", "b@x.org"],
        ])
    );
    let null = IrQuery::new(
        after_optional().project(&["p", "q", "e"]),
        Semantics::sparql().with_missing(Missing::Null3VL),
    );
    let r = run(&t.db.now(), &null);
    assert_eq!(
        sorted(&r),
        expect(&[&["alice", "q1", "a@x.org"], &["bob", "q2", "b@x.org"]])
    );
}

// sql-execution "Absent cell in results"
#[test]
fn absent_cell_under_both_settings() {
    let t = people();
    let op = || {
        Op::left_join(
            b().triple("?p", "v:name", s("Carol")),
            b().triple("?p", "v:age", "?a"),
            None,
        )
    };
    for sem in [Semantics::sparql(), Semantics::cypher()] {
        let r = run(&t.db.now(), &IrQuery::new(op(), sem));
        assert_eq!(
            r.rows,
            vec![vec![Some(ResultValue::Term(v("carol"))), None]]
        );
    }
}

fn ages() -> Op {
    Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "v:age", "?age"),
        None,
    )
}

// sql-execution "Comparison with a missing value" / "Disjunction rescues unknown" /
// "Bound test"
#[test]
fn filter_truth_values() {
    let t = people();
    let gt30 = || Expr::gt(Expr::var("age"), Expr::val(Value::Int(29)));
    let q = b().query(ages().filter(gt30()).project(&["p"]));
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["alice"]]));
    let q = b().query(ages().filter(Expr::not(gt30())).project(&["p"]));
    assert_eq!(
        rows(&run(&t.db.now(), &q)),
        expect(&[&["bob"]]),
        "NOT unknown is unknown"
    );
    let q = b().query(
        ages()
            .filter(Expr::Or(vec![
                gt30(),
                Expr::eq(Expr::var("n"), Expr::val(s("Carol"))),
            ]))
            .project(&["p"]),
    );
    assert_eq!(
        sorted(&run(&t.db.now(), &q)),
        expect(&[&["alice"], &["carol"]])
    );
    let q = b().query(
        ages()
            .filter(Expr::not(Expr::Bound(Var::new("age"))))
            .project(&["p"]),
    );
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["carol"]]));
}

fn values1(var: &str, vals: &[Value]) -> Op {
    Op::Values(Values {
        vars: vec![Var::new(var)],
        rows: vals
            .iter()
            .map(|v| vec![Some(TermOrVar::Const(v.clone()))])
            .collect(),
    })
}

/// Stores `vals` as objects of `v:val` and returns the store.
fn stored(vals: &[Value]) -> TestDb {
    let t = TestDb::new();
    t.tx(|tx| {
        for (i, x) in vals.iter().enumerate() {
            tx.assert(v(&format!("s{i}")), v("val"), x.clone(), Valid::ALWAYS)?;
        }
        Ok(())
    });
    t
}

fn filtered(t: &TestDb, cond: Expr) -> Vec<Vec<String>> {
    let q = b().query(b().triple("?s", "v:val", "?x").filter(cond).project(&["x"]));
    sorted(&run(&t.db.now(), &q))
}

// sql-execution "Incompatible ordering comparison"
#[test]
fn incompatible_ordering_is_unknown() {
    let t = stored(&[s("abc"), Value::Int(3)]);
    let lt5 = || Expr::lt(Expr::var("x"), Expr::val(Value::Int(5)));
    assert_eq!(filtered(&t, lt5()), expect(&[&["3"]]));
    assert_eq!(filtered(&t, Expr::not(lt5())), expect(&[]));
}

// sql-execution "Integer equals decimal" / "String is not an IRI"
#[test]
fn value_equality() {
    let t = stored(&[Value::Int(1), Value::iri("http://ex/a"), s("http://ex/a")]);
    assert_eq!(
        filtered(
            &t,
            Expr::eq(Expr::var("x"), Expr::val(Value::Decimal("1.0".into())))
        ),
        expect(&[&["1"]])
    );
    assert_eq!(
        filtered(&t, Expr::eq(Expr::var("x"), Expr::val(s("http://ex/a")))),
        expect(&[&["http://ex/a"]])
    );
    // different kinds are unequal (false, not unknown): != keeps them
    assert_eq!(
        filtered(&t, Expr::ne(Expr::var("x"), Expr::val(s("http://ex/a")))),
        expect(&[&["1"], &["<http://ex/a>"]])
    );
    // two stored numbers of different kinds compare by value
    let u = stored(&[
        Value::Int(2),
        Value::Double(2.0),
        Value::Decimal("2.5".into()),
    ]);
    let q = b().query(
        Op::join(vec![
            b().triple("?s", "v:val", "?x"),
            b().triple("?t", "v:val", "?y"),
        ])
        .filter(Expr::eq(Expr::var("x"), Expr::var("y")))
        .filter(Expr::ne(Expr::var("s"), Expr::var("t")))
        .project(&["x", "y"]),
    );
    assert_eq!(
        sorted(&run(&u.db.now(), &q)),
        expect(&[&["2", "2"], &["2", "2"]])
    );
}

// design D4: `?x = 1.0` against INT 1 stays a value filter; a datetime constant is
// pushed down as an instant range; sameTerm with a datetime is id equality
#[test]
fn filter_push_down() {
    let t = stored(&[
        Value::Int(1),
        dt("2026-03-01T10:00:00Z"),
        dt("2026-03-01T13:00:00+02:00"),
    ]);
    let eq_dec = b().query(b().triple("?s", "v:val", "?x").filter(Expr::eq(
        Expr::var("x"),
        Expr::val(Value::Decimal("1.0".into())),
    )));
    let ex = explain(&t.db.now(), &eq_dec);
    let sql = ex.sql.unwrap();
    assert!(sql.contains("tm_kind(t0.o)"), "a value filter: {sql}");
    assert_eq!(run(&t.db.now(), &eq_dec).len(), 1);
    let ms = 1_772_359_200_000i64; // 2026-03-01T10:00:00Z
    let eq_dt = b().query(b().triple("?s", "v:val", "?x").filter(Expr::eq(
        Expr::var("x"),
        Expr::val(dt("2026-03-01T12:00:00+02:00")),
    )));
    let ex = explain(&t.db.now(), &eq_dt);
    let sql = ex.sql.clone().unwrap();
    assert!(
        sql.contains("t0.o BETWEEN ?") && sql.contains("(t0.o & 15) = 7"),
        "{sql}"
    );
    assert!(ex.params.contains(&SqlValue::Integer((ms << 15) | 7)));
    assert!(ex.params.contains(&SqlValue::Integer((ms << 15) | 0x7FF7)));
    assert_eq!(
        rows(&run(&t.db.now(), &eq_dt)),
        expect(&[&["s1", "2026-03-01T10:00:00Z"]])
    );
    let same = b().query(b().triple("?s", "v:val", "?x").filter(Expr::SameTerm(
        Box::new(Expr::var("x")),
        Box::new(Expr::val(dt("2026-03-01T10:00:00Z"))),
    )));
    let sql = explain(&t.db.now(), &same).sql.unwrap();
    assert!(
        sql.contains("t0.o = ?") && !sql.contains("BETWEEN"),
        "{sql}"
    );
    assert_eq!(run(&t.db.now(), &same).len(), 1);
}

// sql-execution "Range over long strings" / "Double range"
#[test]
fn range_comparisons() {
    let t = stored(&[s("Alexander"), s("Zoe")]);
    assert_eq!(
        filtered(&t, Expr::lt(Expr::var("x"), Expr::val(s("Mzzzzzzzzz")))),
        expect(&[&["Alexander"]])
    );
    let d = stored(&[Value::Double(0.25), Value::Double(0.5), Value::Double(2.0)]);
    assert_eq!(
        filtered(&d, Expr::ge(Expr::var("x"), Expr::val(Value::Double(0.5)))),
        expect(&[&["0.5"], &["2"]])
    );
}

// sql-execution "Same instant, different offsets"
// (lat.md/tests#ObjectId#DateTime Keeps Its Offset; its @lat reference stays on the M0 codec test)
#[test]
fn same_instant_different_offsets() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(
            v("m"),
            v("a"),
            dt("2026-03-01T12:00:00+02:00"),
            Valid::ALWAYS,
        )?;
        tx.assert(v("m"), v("b"), dt("2026-03-01T10:00:00Z"), Valid::ALWAYS)?;
        Ok(())
    });
    let pair = || b().bgp(&[("v:m", "v:a", "?a"), ("v:m", "v:b", "?b")]);
    let keep = |e: Expr| run(&t.db.now(), &b().query(pair().filter(e))).len();
    let (a, bb) = (|| Expr::var("a"), || Expr::var("b"));
    assert_eq!(keep(Expr::eq(a(), bb())), 1);
    assert_eq!(keep(Expr::lt(a(), bb())), 0);
    assert_eq!(keep(Expr::gt(a(), bb())), 0);
    assert_eq!(keep(Expr::SameTerm(Box::new(a()), Box::new(bb()))), 0);
    let q = b().query(
        Op::union(vec![
            b().triple("v:m", "v:a", "?t"),
            b().triple("v:m", "v:b", "?t"),
        ])
        .project_distinct(&["t"]),
    );
    let r = run(&t.db.now(), &q);
    assert_eq!(
        sorted(&r),
        expect(&[&["2026-03-01T10:00:00Z"], &["2026-03-01T12:00:00+02:00"]])
    );
}

// sql-execution "Datetime constant matches any offset"
#[test]
fn datetime_constant_matches_any_offset() {
    let t = TestDb::new();
    t.tx(|tx| {
        for i in 0..200 {
            let lex = format!("2025-01-01T00:00:{:02}Z", i % 60);
            tx.assert(v(&format!("e{i}")), v("at"), dt(&lex), Valid::ALWAYS)?;
        }
        tx.assert(
            v("meeting"),
            v("at"),
            dt("2026-03-01T10:00:00Z"),
            Valid::ALWAYS,
        )?;
        Ok(())
    });
    t.db.optimize().unwrap();
    let q = b().query(b().triple("?e", "v:at", "?t").filter(Expr::eq(
        Expr::var("t"),
        Expr::val(dt("2026-03-01T12:00:00+02:00")),
    )));
    assert_eq!(
        rows(&run(&t.db.now(), &q)),
        expect(&[&["meeting", "2026-03-01T10:00:00Z"]])
    );
    let plan = explain(&t.db.now(), &q).query_plan.join("\n");
    assert!(
        plan.contains("SEARCH t0") && plan.contains("o>? AND o<?"),
        "{plan}"
    );
}

fn ordered(vals: &[Value], desc: bool, sem: Semantics) -> Vec<String> {
    let t = TestDb::new();
    let q = IrQuery::new(
        values1("x", vals).order_limit(
            vec![if desc {
                Key::desc(Expr::var("x"))
            } else {
                Key::asc(Expr::var("x"))
            }],
            None,
            None,
        ),
        sem,
    );
    rows(&run(&t.db.now(), &q))
        .into_iter()
        .map(|r| r[0].clone())
        .collect()
}

// sql-execution "Strings in and out of the dictionary" / "Doubles" / "Mixed integers
// and doubles" / "Negative datetimes" / "Descending"
// @lat: [[tests#Query#Order By Decoded Value]]
#[test]
fn ordering_by_decoded_value() {
    let sp = Semantics::sparql();
    assert_eq!(
        ordered(&[s("Zoe"), s("Alexander"), s("Bo")], false, sp),
        ["Alexander", "Bo", "Zoe"]
    );
    assert_eq!(
        ordered(
            &[
                Value::Double(10.5),
                Value::Double(-2.0),
                Value::Double(3.25)
            ],
            false,
            sp
        ),
        ["-2", "3.25", "10.5"]
    );
    assert_eq!(
        ordered(
            &[Value::Int(3), Value::Double(2.5), Value::Int(-7)],
            false,
            sp
        ),
        ["-7", "2.5", "3"]
    );
    assert_eq!(
        ordered(
            &[
                dt("1971-01-01T00:00:00Z"),
                dt("1969-06-01T00:00:00Z"),
                dt("1900-01-01T00:00:00Z")
            ],
            false,
            sp
        ),
        [
            "1900-01-01T00:00:00Z",
            "1969-06-01T00:00:00Z",
            "1971-01-01T00:00:00Z"
        ]
    );
    assert_eq!(
        ordered(&[s("a"), s("c"), s("b")], true, sp),
        ["c", "b", "a"]
    );
    // the fixed cross-kind order: IRIs before numbers before strings
    assert_eq!(
        ordered(&[s("z"), Value::Int(1), Value::iri("urn:x")], false, sp),
        ["<urn:x>", "1", "z"]
    );
}

// sql-execution "Missing values placement"
#[test]
fn missing_values_placement() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("a"), v("name"), s("a"), Valid::ALWAYS)?;
        tx.assert(v("a"), v("age"), Value::Int(30), Valid::ALWAYS)?;
        tx.assert(v("b"), v("name"), s("b"), Valid::ALWAYS)?;
        tx.assert(v("c"), v("name"), s("c"), Valid::ALWAYS)?;
        tx.assert(v("c"), v("age"), Value::Int(20), Valid::ALWAYS)?;
        Ok(())
    });
    let op = || {
        Op::left_join(
            b().triple("?p", "v:name", "?n"),
            b().triple("?p", "v:age", "?age"),
            None,
        )
        .order_limit(vec![Key::asc(Expr::var("age"))], None, None)
        .project(&["age"])
    };
    let unbound = run(&t.db.now(), &IrQuery::new(op(), Semantics::sparql()));
    assert_eq!(rows(&unbound), expect(&[&["-"], &["20"], &["30"]]));
    let null = run(&t.db.now(), &IrQuery::new(op(), Semantics::cypher()));
    assert_eq!(rows(&null), expect(&[&["20"], &["30"], &["-"]]));
}

fn staff() -> TestDb {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("zoe"), v("name"), s("Zoe"), Valid::ALWAYS)?;
        tx.assert(v("alexander"), v("name"), s("Alexander"), Valid::ALWAYS)?;
        tx.assert(v("bo"), v("name"), s("Bo"), Valid::ALWAYS)?;
        tx.assert(v("zoe"), v("age"), Value::Int(20), Valid::ALWAYS)?;
        tx.assert(v("bo"), v("age"), Value::Int(30), Valid::ALWAYS)?;
        tx.assert(v("zoe"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("bo"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("alexander"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
        tx.assert(v("zoe"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
        tx.assert(v("zoe"), v("email"), s("z@x"), Valid::ALWAYS)?;
        Ok(())
    });
    t
}

// sql-execution "Min over strings uses value order" / "Average of integers" /
// "Count distinct" / "Collect" / "Grouping with missing keys"
#[test]
fn aggregates() {
    let t = staff();
    let q = b().query(
        b().triple("?p", "v:name", "?n")
            .aggregate(&[], vec![Agg::new("m", AggFunc::Min, Expr::var("n"))]),
    );
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["Alexander"]]));
    let q = b().query(
        Op::left_join(
            b().triple("?p", "v:name", "?n"),
            b().triple("?p", "v:age", "?a"),
            None,
        )
        .aggregate(&[], vec![Agg::new("avg", AggFunc::Avg, Expr::var("a"))]),
    );
    let r = run(&t.db.now(), &q);
    assert_eq!(r.get(0, "avg"), Some(&Value::Double(25.0)));
    let q = b().query(b().triple("?p", "v:worksAt", "?c").aggregate(
        &[],
        vec![Agg::new("n", AggFunc::Count, Expr::var("c")).distinct()],
    ));
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["2"]]));
    let q = b().query(b().triple("?p", "v:worksAt", "?c").aggregate(
        &["p"],
        vec![Agg::new("cs", AggFunc::Collect, Expr::var("c"))],
    ));
    let mut got = sorted(&run(&t.db.now(), &q));
    for r in &mut got {
        let mut l: Vec<String> = r[1]
            .trim_matches(['[', ']'])
            .split(',')
            .map(str::to_string)
            .collect();
        l.sort();
        r[1] = l.join(",");
    }
    assert_eq!(
        got,
        expect(&[
            &["alexander", "globex"],
            &["bo", "acme"],
            &["zoe", "acme,globex"]
        ])
    );
    let q = b().query(
        Op::left_join(
            b().triple("?p", "v:name", "?n"),
            b().triple("?p", "v:email", "?e"),
            None,
        )
        .aggregate(&["e"], vec![Agg::count_star("k")]),
    );
    assert_eq!(
        sorted(&run(&t.db.now(), &q)),
        expect(&[&["-", "2"], &["z@x", "1"]])
    );
    // sum, max, sample, group_concat, HAVING
    let q = b().query(b().triple("?p", "v:age", "?a").aggregate(
        &[],
        vec![
            Agg::new("s", AggFunc::Sum, Expr::var("a")),
            Agg::new("mx", AggFunc::Max, Expr::var("a")),
            Agg::new("any", AggFunc::Sample, Expr::var("a")),
            Agg::new(
                "g",
                AggFunc::GroupConcat { sep: "|".into() },
                Expr::var("a"),
            ),
        ],
    ));
    let r = run(&t.db.now(), &q);
    assert_eq!(r.get(0, "s"), Some(&Value::Int(50)));
    assert_eq!(r.get(0, "mx"), Some(&Value::Int(30)));
    assert!(r.get(0, "any").is_some());
    let g = short(r.get(0, "g").unwrap());
    assert!(g == "20|30" || g == "30|20", "{g}");
    let q = b().query(
        b().triple("?p", "v:worksAt", "?c")
            .aggregate(&["c"], vec![Agg::count_star("n")])
            .filter(Expr::gt(Expr::var("n"), Expr::val(Value::Int(1)))),
    );
    let ex = explain(&t.db.now(), &q);
    assert!(ex.sql.unwrap().contains("HAVING"));
    assert_eq!(
        sorted(&run(&t.db.now(), &q)),
        expect(&[&["acme", "2"], &["globex", "2"]])
    );
}

// expressions: arithmetic, functions, IN, COALESCE, IF
#[test]
fn expressions() {
    let t = staff();
    let q = b().query(
        b().triple("?p", "v:age", "?a")
            .extend(
                "next",
                Expr::Arith(
                    ArithOp::Add,
                    Box::new(Expr::var("a")),
                    Box::new(Expr::val(Value::Int(1))),
                ),
            )
            .extend(
                "half",
                Expr::Arith(
                    ArithOp::Div,
                    Box::new(Expr::var("a")),
                    Box::new(Expr::val(Value::Int(4))),
                ),
            )
            .filter(Expr::In(
                Box::new(Expr::var("a")),
                vec![Expr::val(Value::Int(20)), Expr::val(Value::Int(99))],
                false,
            ))
            .project(&["p", "next", "half"]),
    );
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["zoe", "21", "5"]]));
    let q = b().query(
        b().triple("?p", "v:name", "?n")
            .extend("u", Expr::Func(Func::UCase, vec![Expr::var("n")]))
            .extend("l", Expr::Func(Func::StrLen, vec![Expr::var("n")]))
            .filter(Expr::Func(
                Func::StrStarts,
                vec![Expr::var("n"), Expr::val(s("Al"))],
            ))
            .filter(Expr::Func(
                Func::Regex,
                vec![Expr::var("n"), Expr::val(s("^al")), Expr::val(s("i"))],
            ))
            .project(&["u", "l"]),
    );
    assert_eq!(rows(&run(&t.db.now(), &q)), expect(&[&["ALEXANDER", "9"]]));
    let q = b().query(
        Op::left_join(
            b().triple("?p", "v:name", "?n"),
            b().triple("?p", "v:email", "?e"),
            None,
        )
        .extend("c", Expr::Coalesce(vec![Expr::var("e"), Expr::var("n")]))
        .extend("isLit", Expr::Func(Func::IsLiteral, vec![Expr::var("n")]))
        .extend("isIri", Expr::Func(Func::IsIri, vec![Expr::var("p")]))
        .extend(
            "k",
            Expr::If(
                Box::new(Expr::Bound(Var::new("e"))),
                Box::new(Expr::val(s("yes"))),
                Box::new(Expr::val(s("no"))),
            ),
        )
        .project(&["p", "c", "isLit", "isIri", "k"]),
    );
    assert_eq!(
        sorted(&run(&t.db.now(), &q)),
        expect(&[
            &["alexander", "Alexander", "true", "true", "no"],
            &["bo", "Bo", "true", "true", "no"],
            &["zoe", "z@x", "true", "true", "yes"],
        ])
    );
}
