//! Spec `sql-execution` "Typed result decoding" and Triangle / cyclic routing.

mod common;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Agg, AggFunc, Expr, Op};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

// "Every ObjectId kind decodes"
#[test]
fn every_kind_round_trips() {
    let t = TestDb::new();
    let xsd = "http://www.w3.org/2001/XMLSchema#";
    let vals = vec![
        v("iri"),
        Value::Node(7),
        Value::BNode(8),
        Value::Stmt(Eid::new(9)),
        Value::Tx(TxId(3)),
        Value::Int(-42),
        Value::Bool(true),
        Value::DateTime {
            ms: 1_772_359_200_000,
            tz: Some(120),
        },
        Value::DateTime {
            ms: -5_000,
            tz: None,
        },
        Value::DateTime { ms: 0, tz: Some(0) },
        Value::Date(-1),
        s("short"),
        s("a much longer plain string"),
        Value::LangStr {
            lex: "chat".into(),
            lang: "fr".into(),
        },
        Value::literal("P3D", Some(&format!("{xsd}duration")), None),
        Value::Double(2.5),
        Value::Decimal("10.25".into()),
    ];
    t.tx(|tx| {
        for (i, x) in vals.iter().enumerate() {
            tx.assert(v(&format!("s{i}")), v("val"), x.clone(), Valid::ALWAYS)?;
        }
        Ok(())
    });
    let q = b().query(b().triple("?s", "v:val", "?o").project(&["o"]));
    let r = run(&t.db.now(), &q);
    let got: Vec<Value> = r.column("o").into_iter().map(|c| c.unwrap()).collect();
    assert_eq!(got.len(), vals.len());
    for want in &vals {
        assert!(
            got.contains(&want.canonical()),
            "{want:?} missing from {got:?}"
        );
    }
    // a TYPED literal keeps its datatype
    assert!(got.contains(&Value::Typed {
        lex: "P3D".into(),
        datatype: format!("{xsd}duration")
    }));
}

// "Computed values"
#[test]
fn computed_values_and_lists() {
    let t = TestDb::new();
    t.tx(|tx| {
        for (i, x) in [1.5, 2.5, 3.5].iter().enumerate() {
            tx.assert(
                v(&format!("s{i}")),
                v("score"),
                Value::Double(*x),
                Valid::ALWAYS,
            )?;
            tx.assert(v(&format!("s{i}")), v("group"), v("g"), Valid::ALWAYS)?;
        }
        Ok(())
    });
    let q = b().query(
        b().bgp(&[("?s", "v:score", "?x"), ("?s", "v:group", "?g")])
            .aggregate(
                &["g"],
                vec![
                    Agg::count_star("n"),
                    Agg::new("avg", AggFunc::Avg, Expr::var("x")),
                    Agg::new("all", AggFunc::Collect, Expr::var("x")),
                ],
            ),
    );
    let r = run(&t.db.now(), &q);
    assert_eq!(r.get(0, "n"), Some(&Value::Int(3)));
    assert_eq!(r.get(0, "avg"), Some(&Value::Double(2.5)));
    let list = r.rows[0][r.col("all").unwrap()]
        .as_ref()
        .unwrap()
        .as_list()
        .unwrap();
    let mut xs: Vec<f64> = list
        .iter()
        .map(|c| match c.as_ref().unwrap().as_term().unwrap() {
            Value::Double(x) => *x,
            other => panic!("{other:?}"),
        })
        .collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(xs, [1.5, 2.5, 3.5]);
}

// Triangle: a cyclic BGP returns the brute-force result and explain reports the route
#[test]
fn triangle_is_cyclic_and_correct() {
    let t = TestDb::new();
    let n = 12usize;
    let mut edges = std::collections::BTreeSet::new();
    for i in 0..n {
        for k in [1usize, 2, 5] {
            edges.insert((i, (i * 3 + k) % n));
        }
    }
    t.tx(|tx| {
        for (a, c) in &edges {
            tx.assert(
                v(&format!("n{a}")),
                v("knows"),
                v(&format!("n{c}")),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    });
    let q = b().query(b().bgp(&[
        ("?a", "v:knows", "?b"),
        ("?b", "v:knows", "?c"),
        ("?c", "v:knows", "?a"),
    ]));
    let ex = explain(&t.db.now(), &q);
    assert!(ex
        .regions
        .iter()
        .any(|r| r.note == RouteNote::CyclicLftjDisabled && r.kind == RegionKind::Sql));
    let mut brute = Vec::new();
    for &(a, bb) in &edges {
        for &(b2, c) in &edges {
            if b2 == bb && edges.contains(&(c, a)) {
                brute.push(vec![format!("n{a}"), format!("n{bb}"), format!("n{c}")]);
            }
        }
    }
    brute.sort();
    assert!(!brute.is_empty());
    assert_eq!(sorted(&run(&t.db.now(), &q)), brute);
    // acyclic patterns carry no note
    let chain = b().query(b().bgp(&[("?a", "v:knows", "?b"), ("?b", "v:knows", "?c")]));
    assert!(explain(&t.db.now(), &chain)
        .regions
        .iter()
        .all(|r| r.note == RouteNote::None));
    let _ = Op::unit();
}
