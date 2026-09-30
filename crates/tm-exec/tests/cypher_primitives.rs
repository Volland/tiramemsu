//! Spec `query-ir`: list unnesting, correlated property lookup, null-safe join keys
//! and row numbering (the Cypher support primitives).

mod common;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{
    Agg, AggFunc, Expr, Join, Key, Lookup, LookupMode, Op, RowNumber, Unnest, Values, View,
};
use tiramemsu::ir::{Semantics, TermOrVar, Var};
use tiramemsu::*;

fn b() -> IrBuilder {
    IrBuilder::cypher()
}

fn unnest(input: Op, list: Expr, var: &str) -> Op {
    Op::Unnest(Unnest {
        input: Box::new(input),
        list,
        var: Var::new(var),
    })
}

fn ints(xs: &[i64]) -> Expr {
    Expr::List(xs.iter().map(|x| Expr::val(Value::Int(*x))).collect())
}

// "Unwind a constant list"
#[test]
fn unwind_constant_list_keeps_order() {
    let t = TestDb::new();
    let q = b().query(unnest(Op::unit(), ints(&[3, 1, 2]), "x"));
    assert_eq!(
        rows(&run(&t.db.now(), &q)),
        expect(&[&["3"], &["1"], &["2"]])
    );
    let q = b().query(unnest(Op::unit(), ints(&[]), "x"));
    let r = run(&t.db.now(), &q);
    assert!(r.is_empty());
    assert_eq!(cols(&r), ["x"]);
}

// "Unwind a computed list per row"
#[test]
fn unwind_computed_list_per_row() {
    let t = TestDb::new();
    t.tx(|tx| {
        tx.assert(v("alice"), v("name"), s("Alice"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("name"), s("Bob"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
        Ok(())
    });
    let lists = Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "v:worksAt", "?c"),
        None,
    )
    .aggregate(
        &["p"],
        vec![Agg::new("l", AggFunc::Collect, Expr::var("c"))],
    );
    let r = run(&t.db.now(), &b().query(lists.clone()));
    assert_eq!(r.len(), 2, "bob has an empty list");
    let q = b().query(unnest(lists, Expr::var("l"), "x").project(&["p", "x"]));
    let r = run(&t.db.now(), &q);
    assert_eq!(
        sorted(&r),
        expect(&[&["alice", "acme"], &["alice", "globex"]])
    );
    // a missing list yields no rows
    let missing = Op::left_join(
        b().triple("?p", "v:name", "?n"),
        b().triple("?p", "v:nothing", "?l"),
        None,
    );
    assert!(run(
        &t.db.now(),
        &b().query(unnest(missing, Expr::var("l"), "x"))
    )
    .is_empty());
}

fn lookup(subject: &str, pred: &str, multi: LookupMode) -> Expr {
    Expr::Lookup(Lookup {
        subject: Box::new(Expr::var(subject)),
        pred: TermOrVar::iri(vi(pred)),
        view: View::NOW,
        multi,
        include_volatile: false,
    })
}

// "Single-valued lookup" / "Missing and multi-valued lookup"
#[test]
fn property_lookup() {
    let t = TestDb::new();
    t.tx(|tx| {
        for p in ["alice", "bob", "carol"] {
            tx.assert(v(p), v("type"), v("Person"), Valid::ALWAYS)?;
        }
        tx.assert(v("alice"), v("name"), s("Alice"), Valid::ALWAYS)?;
        tx.assert(v("carol"), v("name"), s("Carol B"), Valid::ALWAYS)?;
        tx.assert(v("carol"), v("name"), s("Carol A"), Valid::ALWAYS)?;
        Ok(())
    });
    let people = || b().triple("?p", "v:type", "v:Person");
    let single = b().query(
        people()
            .extend("n", lookup("p", "name", LookupMode::Single))
            .project(&["p", "n"]),
    );
    assert_eq!(
        sorted(&run(&t.db.now(), &single)),
        expect(&[&["alice", "Alice"], &["bob", "-"], &["carol", "Carol B"]]),
        "smallest eid wins; one row each"
    );
    let list = b().query(
        people()
            .extend("n", lookup("p", "name", LookupMode::ListIfMany))
            .project(&["p", "n"]),
    );
    assert_eq!(
        sorted(&run(&t.db.now(), &list)),
        expect(&[
            &["alice", "Alice"],
            &["bob", "-"],
            &["carol", "[Carol B,Carol A]"]
        ]),
        "eid order, still one row each"
    );
}

fn undef_values(vars: &[&str], rows: Vec<Vec<Option<i64>>>) -> Op {
    Op::Values(Values {
        vars: vars.iter().map(Var::new).collect(),
        rows: rows
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|c| c.map(|i| TermOrVar::Const(Value::Int(i))))
                    .collect()
            })
            .collect(),
    })
}

// "Null-safe decorrelation join"
#[test]
fn null_safe_join_keys() {
    let t = TestDb::new();
    let inputs = || {
        vec![
            undef_values(
                &["a", "x"],
                vec![vec![None, Some(1)], vec![Some(7), Some(2)]],
            ),
            undef_values(
                &["a", "y"],
                vec![vec![None, Some(10)], vec![Some(7), Some(20)]],
            ),
        ]
    };
    let q = |safe: bool| {
        IrQuery::new(
            Op::Join(Join {
                inputs: inputs(),
                null_safe: if safe { vec![Var::new("a")] } else { vec![] },
            }),
            Semantics::cypher(),
        )
    };
    let plain = run(&t.db.now(), &q(false));
    assert_eq!(
        sorted(&plain),
        expect(&[&["7", "2", "20"]]),
        "missing never joins unmarked"
    );
    let safe = run(&t.db.now(), &q(true));
    assert_eq!(
        sorted(&safe),
        expect(&[&["-", "1", "10"], &["7", "2", "20"]])
    );
}

// "Top-1 per group"
#[test]
fn row_number_top_one_per_group() {
    let t = TestDb::new();
    t.tx(|tx| {
        for (p, a) in [("alice", 30), ("alice", 40), ("bob", 20)] {
            tx.create(v(p), v("age"), Value::Int(a), Valid::ALWAYS)?;
        }
        Ok(())
    });
    let numbered = Op::RowNumber(RowNumber {
        input: Box::new(b().triple("?p", "v:age", "?a")),
        partition: vec![Var::new("p")],
        order: vec![Key::desc(Expr::var("a"))],
        var: Var::new("n"),
    });
    let q = b().query(
        numbered
            .filter(Expr::Cmp(
                tiramemsu::ir::CmpOp::Le,
                Box::new(Expr::var("n")),
                Box::new(Expr::val(Value::Int(1))),
            ))
            .project(&["p", "a"]),
    );
    assert_eq!(
        sorted(&run(&t.db.now(), &q)),
        expect(&[&["alice", "40"], &["bob", "20"]])
    );
}
