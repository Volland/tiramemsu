//! The `tm_path` table function and the `View::path` API (`path-table-function`).

mod common;

use std::sync::Arc;

use common::paths::*;
use common::*;
use tiramemsu::*;

fn q(g: &G, sql: &str) -> Vec<Vec<SqlValue>> {
    g.t.db
        .read_sql(sql)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

fn err(g: &G, sql: &str) -> String {
    match g.t.db.read_sql(sql) {
        Err(Error::Sqlite(e)) => e.message,
        other => panic!("{sql}: {other:?}"),
    }
}

fn ints(rows: &[Vec<SqlValue>], col: usize) -> Vec<i64> {
    rows.iter().map(|r| r[col].as_i64().unwrap()).collect()
}

fn chain20(g: &G) {
    let names: Vec<String> = (0..=20).map(|i| format!("n{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    chain(g, "next", &refs);
}

// "Callable on a reader connection" / "Minimal call" / "Select star columns" / "Writes are rejected"
#[test]
fn callable_everywhere_and_read_only() {
    let g = G::new();
    g.edge("alice", "knows", "bob");
    let a = g.id("alice").raw();
    let rows = q(
        &g,
        &format!("SELECT * FROM tm_path({a}, 'knows+', 'REACH')"),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].len(), 5, "start, end, hops, path_json, arrival");
    assert_eq!(rows[0][1].as_i64(), Some(g.id("bob").raw()));
    assert!(rows[0][3].is_null());
    assert!(rows[0][4].is_null(), "no arrival without timeRespecting");
    let cols = q(&g, "SELECT name FROM pragma_table_info('tm_path')");
    let names: Vec<_> = cols
        .iter()
        .map(|r| r[0].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["start", "end", "hops", "path_json", "arrival"]);
    let minimal = q(&g, &format!("SELECT \"end\" FROM tm_path({a}, 'knows')"));
    assert_eq!(ints(&minimal, 0), vec![g.id("bob").raw()]);
    assert!(g
        .t
        .db
        .read_sql("INSERT INTO tm_path VALUES (1, 2, 3, NULL)")
        .is_err());
    assert_eq!(
        q(&g, &format!("SELECT count(*) FROM tm_path({a}, 'knows')"))[0][0].as_i64(),
        Some(1)
    );
}

// "Path value contents" (API) / "path_json for a trail" / "Zero-length row" / JSON round trip
#[test]
fn path_values_and_json() {
    let g = G::new();
    let e = g.edges(&[("a", "knows", "b"), ("c", "knows", "b")]);
    let view = g.t.db.now();
    let rows = view.path(g.id("a"), "knows/^knows", TRAIL, 5).unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!((r.start, r.end, r.hops), (g.id("a"), g.id("c"), 2));
    let p = r.path.as_ref().unwrap();
    assert_eq!(p.nodes, vec![g.id("a"), g.id("b"), g.id("c")]);
    assert_eq!(
        p.hops
            .iter()
            .map(|h| (h.eid, h.pred, h.dir))
            .collect::<Vec<_>>(),
        vec![
            (e[0].oid(), g.id("knows"), PathDir::Out),
            (e[1].oid(), g.id("knows"), PathDir::In)
        ]
    );
    // API "returns path values"
    let one = view.path(g.id("a"), "knows", TRAIL, 1).unwrap();
    assert_eq!(
        one[0].path.as_ref().unwrap().nodes,
        vec![g.id("a"), g.id("b")]
    );
    assert_eq!(one[0].path.as_ref().unwrap().hops[0].eid, e[0].oid());
    // path_json parses and equals the triple's eid
    let a = g.id("a").raw();
    let sql = format!(
        "SELECT p.path_json, j.value ->> 'eid', j.value ->> 'dir', json_array_length(p.path_json, '$.nodes') \
         FROM tm_path({a}, 'knows', 'TRAIL') p, json_each(p.path_json, '$.edges') j"
    );
    let rows = q(&g, &sql);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1].as_i64(), Some(e[0].oid().raw()));
    assert_eq!(rows[0][2].as_str(), Some("out"));
    assert_eq!(rows[0][3].as_i64(), Some(2));
    let expect = format!(
        "{{\"nodes\":[{a},{}],\"edges\":[{{\"eid\":{},\"p\":{},\"dir\":\"out\"}}]}}",
        g.id("b").raw(),
        e[0].oid().raw(),
        g.id("knows").raw()
    );
    assert_eq!(rows[0][0].as_str(), Some(expect.as_str()));
    // "path_json is NULL in REACH mode" / "Zero-length row"
    let reach = q(
        &g,
        &format!("SELECT path_json FROM tm_path({a}, 'knows+', 'REACH')"),
    );
    assert!(reach.iter().all(|r| r[0].is_null()));
    let zero = q(&g, &format!("SELECT start, \"end\", hops, path_json FROM tm_path({a}, 'knows*', 'TRAIL') WHERE hops = 0"));
    assert_eq!(zero.len(), 1);
    assert_eq!(zero[0][2].as_i64(), Some(0));
    assert_eq!(
        zero[0][3].as_str(),
        Some(format!("{{\"nodes\":[{a}],\"edges\":[]}}").as_str())
    );
}

// "Correlated start from another table" / "Aggregate over paths" / "End constraint" / "JSON inspection of hops"
#[test]
fn composes_with_sql() {
    let g = G::new();
    g.t.tx(|tx| {
        for n in ["ann", "bob"] {
            tx.assert(
                v(n),
                Value::iri(format!("{}type", vocab::RDF)),
                v("Person"),
                Valid::ALWAYS,
            )?;
        }
        tx.assert(v("ann"), v("knows"), v("bob"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("knows"), v("cy"), Valid::ALWAYS)?;
        Ok(())
    });
    let (ty, person) = (
        g.t.id(&Value::iri(format!("{}type", vocab::RDF))).raw(),
        g.id("Person").raw(),
    );
    let sql = format!(
        "SELECT t.s, p.\"end\" FROM triple t, tm_path(t.s, 'knows+', 'REACH') p \
         WHERE t.p = {ty} AND t.o = {person} AND t.t_ret IS NULL ORDER BY 1, 2"
    );
    let rows = q(&g, &sql);
    let pairs: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            (
                g.name(ObjectId::from_raw(r[0].as_i64().unwrap())),
                g.name(ObjectId::from_raw(r[1].as_i64().unwrap())),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [("ann", "bob"), ("ann", "cy"), ("bob", "cy")].map(|(a, b)| (a.to_string(), b.to_string()))
    );
    // aggregate: two shortest paths to d
    let g = G::new();
    g.edges(&[
        ("a", "p", "b"),
        ("a", "p", "c"),
        ("b", "p", "d"),
        ("c", "p", "d"),
    ]);
    let a = g.id("a").raw();
    let agg = q(&g, &format!("SELECT \"end\", count(*) FROM tm_path({a}, 'p+', 'ALL_SHORTEST') GROUP BY \"end\" ORDER BY 2 DESC"));
    assert_eq!(
        (agg[0][0].as_i64(), agg[0][1].as_i64()),
        (Some(g.id("d").raw()), Some(2))
    );
    // end constraint = filtering afterwards
    let d = g.id("d").raw();
    let with_end = q(
        &g,
        &format!("SELECT * FROM tm_path({a}, 'p+', 'ANY_SHORTEST') WHERE \"end\" = {d}"),
    );
    let all: Vec<_> = q(
        &g,
        &format!("SELECT * FROM tm_path({a}, 'p+', 'ANY_SHORTEST')"),
    )
    .into_iter()
    .filter(|r| r[1].as_i64() == Some(d))
    .collect();
    assert_eq!(with_end, all);
    assert_eq!(with_end.len(), 1);
    // JSON inspection: one entry per hop of every trail
    let hops = q(&g, &format!("SELECT j.value ->> 'eid' FROM tm_path({a}, 'p{{1,3}}', 'TRAIL') p, json_each(p.path_json, '$.edges') j"));
    let total: i64 = q(
        &g,
        &format!("SELECT sum(hops) FROM tm_path({a}, 'p{{1,3}}', 'TRAIL')"),
    )[0][0]
        .as_i64()
        .unwrap();
    assert_eq!(hops.len() as i64, total);
}

// "Mode is case-insensitive" / "Default trail cap" / "Explicit max_hops" / "Bounded repetition suffix"
#[test]
fn arguments_and_defaults() {
    let g = G::new();
    chain20(&g);
    let n0 = g.id("n0").raw();
    let count = |args: &str| {
        q(&g, &format!("SELECT count(*) FROM tm_path({n0}, {args})"))[0][0]
            .as_i64()
            .unwrap()
    };
    assert_eq!(count("'next+', 'TRAIL'"), 15);
    assert_eq!(count("'next+', 'TRAIL', 18"), 18);
    assert_eq!(count("'next+', 'REACH'"), 20, "no cap in REACH");
    assert_eq!(count("'next+', 'reach', 3"), 3);
    let a = q(
        &g,
        &format!("SELECT * FROM tm_path({n0}, 'next+', 'any_shortest')"),
    );
    let b = q(
        &g,
        &format!("SELECT * FROM tm_path({n0}, 'next+', 'ANY_SHORTEST')"),
    );
    assert_eq!(a, b);
    let r = q(
        &g,
        &format!("SELECT \"end\" FROM tm_path({n0}, 'next{{2,3}}', 'TRAIL')"),
    );
    assert_eq!(ints(&r, 0), vec![g.id("n2").raw(), g.id("n3").raw()]);
}

// "tm_path argument errors": the messages start with `tm_path:` and name the argument
#[test]
fn argument_errors() {
    let g = G::new();
    g.edge("a", "p", "b");
    let a = g.id("a").raw();
    let cases = [
        (format!("SELECT * FROM tm_path({a})"), "path"),
        (format!("SELECT * FROM tm_path({a}, 'p', 'WALK')"), "mode"),
        (
            format!("SELECT * FROM tm_path({a}, 'p', 'TRAIL', -1)"),
            "max_hops",
        ),
        (
            format!("SELECT * FROM tm_path({a}, 'p', 'REACH', NULL, 'asOf/yesterday')"),
            "view",
        ),
        ("SELECT * FROM tm_path('alice', 'p')".to_string(), "start"),
        (format!("SELECT * FROM tm_path({a}, 'p//q')"), "path"),
        (format!("SELECT * FROM tm_path({a}, 'nope:q')"), "path"),
        (format!("SELECT * FROM tm_path({a}, 5)"), "path"),
    ];
    for (sql, arg) in cases {
        let m = err(&g, &sql);
        assert!(m.starts_with(&format!("tm_path: {arg}")), "{sql}: {m}");
    }
    assert!(err(&g, &format!("SELECT * FROM tm_path({a}, 'p//q')")).contains("offset 2"));
    // NULL start from an outer join: zero rows, no error
    let rows = q(
        &g,
        "SELECT 1, p.\"end\" FROM (SELECT 1) LEFT JOIN triple t ON t.eid = -5 \
         LEFT JOIN tm_path(t.s, 'p') p",
    );
    assert_eq!(rows.len(), 1);
    assert!(rows[0][1].is_null());
}

// "Bare names resolve through vocab" / "Full IRI and CURIE atoms" / "Wildcard atom"
#[test]
fn path_text_atoms() {
    let g = G::new();
    g.t.tx(|tx| {
        tx.set_prefix("schema", "https://schema.org/")?;
        tx.assert(
            Value::iri("https://x/a"),
            Value::iri("https://schema.org/knows"),
            Value::iri("https://x/b"),
            Valid::ALWAYS,
        )?;
        tx.assert(
            Value::iri("https://x/a"),
            Value::iri("https://schema.org/follows"),
            Value::iri("https://x/c"),
            Valid::ALWAYS,
        )?;
        tx.assert(v("a"), v("SUPPORTED_BY"), v("b"), Valid::ALWAYS)?;
        Ok(())
    });
    let view = g.t.db.now();
    let a = view.encode(&Value::iri("https://x/a")).unwrap().unwrap();
    let both = view
        .path(a, "<https://schema.org/knows>|schema:follows", REACH, 5)
        .unwrap();
    assert_eq!(both.len(), 2);
    let one = view.path(g.id("a"), "SUPPORTED_BY", REACH, 5).unwrap();
    assert_eq!(one.len(), 1);
    // a database `@vocab` changes what bare names mean
    g.t.tx(|tx| {
        tx.set_vocab("https://schema.org/")?;
        Ok(())
    });
    assert_eq!(g.t.db.now().path(a, "knows", REACH, 5).unwrap().len(), 1);
}

// "Concurrent commit is not seen mid-query"
#[test]
fn a_running_path_query_keeps_its_snapshot() {
    let t = Arc::new(TestDb::new());
    t.tx(|tx| {
        tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)
            .map(|_| ())
    });
    t.tx(|tx| {
        tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)
            .map(|_| ())
    });
    let b = tiramemsu::ir::builder::IrBuilder::sparql();
    let query = b.query(b.path(
        tiramemsu::ir::TermOrVar::iri(vi("a")),
        tiramemsu::ir::PathExpr::iri(vi("knows")).plus(),
        "?x",
        REACH,
    ));
    let t2 = t.clone();
    t.db.set_query_hook(Some(Arc::new(move || {
        t2.tx(|tx| {
            tx.assert(v("c"), v("knows"), v("d"), Valid::ALWAYS)
                .map(|_| ())
        });
    })));
    let during = run(&t.db.now(), &query);
    t.db.set_query_hook(None);
    assert_eq!(during.len(), 2, "b and c, not d");
    assert_eq!(run(&t.db.now(), &query).len(), 3);
}

// api "API matches tm_path" / "API parse error"
#[test]
fn api_matches_sql_and_reports_parse_errors() {
    let g = G::new();
    g.t.advance_to(9);
    g.edge("alice", "knows", "bob");
    g.t.advance_to(19);
    g.edge("bob", "knows", "carol");
    let alice = g.id("alice");
    let api: Vec<(i64, i64, i64)> =
        g.t.db
            .as_of(TimeRef::Tx(15))
            .path(alice, "knows+", REACH, u32::MAX)
            .unwrap()
            .iter()
            .map(|r| (r.start.raw(), r.end.raw(), i64::from(r.hops)))
            .collect();
    let sql = q(
        &g,
        &format!(
            "SELECT start, \"end\", hops FROM tm_path({}, 'knows+', 'REACH', NULL, 'asOf/15')",
            alice.raw()
        ),
    );
    let sqlrows: Vec<(i64, i64, i64)> = sql
        .iter()
        .map(|r| {
            (
                r[0].as_i64().unwrap(),
                r[1].as_i64().unwrap(),
                r[2].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(api, sqlrows);
    match g.t.db.now().path(alice, "knows//", REACH, 3) {
        Err(Error::Parse {
            dialect: Dialect::Path,
            span: Some(s),
            ..
        }) => assert_eq!(s.offset, 6),
        other => panic!("{other:?}"),
    }
}
