//! Graph-scoped path evaluation (`add-graph-scoped-paths`): the engine's graph
//! filter, the fetch shape it adds, the `graphs` argument of `tm_path`, and the
//! planner's `PathPattern` graph selector.

mod common;

use common::paths::*;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{GraphSel, Op, PathExpr, TermOrVar};
use tiramemsu::*;
use tm_exec::path::automaton::Dir;
use tm_exec::path::fetch::Fetcher;
use tm_exec::path::resolve::{Fetch, RLetter};
use tm_exec::path::row::HopKind;
use tm_exec::scan::{ResolvedTx, ResolvedView};
use tm_rusqlite::RusqliteExec;

fn gv(name: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:g:{name}"))
}

/// `a knows b` and `b knows c` in g1, `c knows d` in g2, `a knows x` in no graph,
/// `a likes b` in g1 and g2.
fn fixture() -> G {
    let g = G::new();
    g.t.tx(|tx| {
        let ab = tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?.eid();
        let bc = tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)?.eid();
        let cd = tx.assert(v("c"), v("knows"), v("d"), Valid::ALWAYS)?.eid();
        tx.assert(v("a"), v("knows"), v("x"), Valid::ALWAYS)?;
        let likes = tx.assert(v("a"), v("likes"), v("b"), Valid::ALWAYS)?.eid();
        for (e, graph) in [(ab, "1"), (bc, "1"), (cd, "2"), (likes, "1"), (likes, "2")] {
            tx.add_to_graph(e, gv(graph), AssertOpts::default())?;
        }
        Ok(())
    });
    g
}

fn ends_in(
    g: &G,
    view: View<'_>,
    start: &str,
    path: &str,
    mode: PathMode,
    graphs: Option<&[&str]>,
) -> Vec<(String, u32)> {
    let graphs = graphs.map(|gs| {
        gs.iter()
            .filter_map(|n| view.encode(&gv(n)).unwrap())
            .collect::<Vec<_>>()
    });
    let args = PathArgs {
        mode,
        graphs,
        ..PathArgs::default()
    };
    view.path_with(g.id(start), path, &args)
        .unwrap()
        .iter()
        .map(|r| (local(&view.decode(r.end).unwrap()), r.hops))
        .collect()
}

// @lat: [[tests#Named Graphs#Graph Scoped Paths Follow Membership]]
#[test]
fn graph_scoped_paths_follow_membership() {
    let g = fixture();
    let now = || g.t.db.now();
    // confined to g1; widening the set widens the path
    assert_eq!(
        ends_in(&g, now(), "a", "knows+", REACH, Some(&["1"])),
        eh(&[("b", 1), ("c", 2)])
    );
    assert_eq!(
        sorted_vec(&ends_in(&g, now(), "a", "knows+", REACH, Some(&["1", "2"]))),
        eh(&[("b", 1), ("c", 2), ("d", 3)])
    );
    // no filter: the union, including the statement in no graph
    assert_eq!(
        ends_in(&g, now(), "a", "knows+", REACH, None).len(),
        4,
        "b, c, d and x"
    );
    // a statement in two graphs of the set is one hop, not two
    assert_eq!(
        ends_in(&g, now(), "a", "likes", TRAIL, Some(&["1", "2"])),
        eh(&[("b", 1)])
    );
    // zero-hop rows do not depend on the graphs; an unknown graph has no member
    assert_eq!(
        ends_in(&g, now(), "a", "knows*", REACH, Some(&["nope"])),
        eh(&[("a", 0)])
    );
    assert_eq!(
        ends_in(&g, now(), "a", "knows*", ANY, Some(&[])),
        eh(&[("a", 0)])
    );
    // every mode filters: the trail and the shortest paths through g1 only
    for mode in [TRAIL, ANY, ALL] {
        assert_eq!(
            ends_in(&g, now(), "a", "knows+", mode, Some(&["1"])),
            eh(&[("b", 1), ("c", 2)]),
            "{mode:?}"
        );
    }
    // the membership is read in the hop's view: removing it later keeps the past
    let before = g.t.last_t();
    let bc =
        g.t.db
            .now()
            .triples(Some(g.id("b")), Some(g.id("knows")), None)
            .unwrap()[0]
            .eid;
    g.t.tx(|tx| tx.remove_from_graph(bc, gv("1")).map(|_| ()));
    assert_eq!(
        ends_in(&g, now(), "a", "knows+", REACH, Some(&["1"])),
        eh(&[("b", 1)])
    );
    assert_eq!(
        ends_in(
            &g,
            g.t.db.as_of(TimeRef::Tx(before)),
            "a",
            "knows+",
            REACH,
            Some(&["1"])
        ),
        eh(&[("b", 1), ("c", 2)])
    );
    // a hop's statement retracted: history still sees the path, now does not
    assert_eq!(
        ends_in(&g, g.t.db.history(), "a", "knows+", REACH, Some(&["1"])),
        eh(&[("b", 1), ("c", 2)])
    );
}

// @lat: [[tests#Named Graphs#Graph Scoped Virtual Hops]]
#[test]
fn virtual_hops_need_the_stepped_statement_in_the_graph() {
    for (fact_graph, want) in [("1", vec!["acme", "alice"]), ("2", vec![])] {
        let g = G::new();
        g.t.tx(|tx| {
            let e1 = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid();
            let e7 = tx
                .assert(v("belief9"), v("supportedBy"), e1, Valid::ALWAYS)?
                .eid();
            tx.add_to_graph(e1, gv(fact_graph), AssertOpts::default())?;
            tx.add_to_graph(e7, gv("1"), AssertOpts::default())?;
            Ok(())
        });
        let now = g.t.db.now();
        let mut got: Vec<String> = ends_in(
            &g,
            now,
            "belief9",
            "supportedBy/(sys:subject|sys:object)",
            REACH,
            Some(&["1"]),
        )
        .into_iter()
        .map(|(n, _)| n)
        .collect();
        got.sort();
        assert_eq!(got, want, "fact in g{fact_graph}");
        // the inverse virtual hop also needs the statement in the graph
        let back = ends_in(
            &g,
            g.t.db.now(),
            "alice",
            "^sys:subject/^supportedBy",
            REACH,
            Some(&["1"]),
        );
        assert_eq!(back.len(), usize::from(fact_graph == "1"), "{back:?}");
    }
    // a database that never wrote `sys:inGraph`: only zero-hop rows
    let g = G::new();
    g.edge("a", "knows", "b");
    let other = g.id("b"); // any id: no graph exists
    let rows =
        g.t.db
            .now()
            .path_with(
                g.id("a"),
                "knows*",
                &PathArgs {
                    graphs: Some(vec![other]),
                    ..PathArgs::default()
                },
            )
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hops, 0);
}

fn exec_of(t: &TestDb) -> RusqliteExec {
    let conn = rusqlite::Connection::open(&t.path).unwrap();
    tm_rusqlite::register::register_defaults(&conn).unwrap();
    RusqliteExec::from_connection(conn, Capabilities::default())
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
        ("subject in", mk(Fetch::Virtual(HopKind::Subject), Dir::In)),
    ]
}

// @lat: [[tests#Named Graphs#Graph Filter Fetch Uses An Index Seek]]
#[test]
fn graph_filter_uses_an_index_seek() {
    let g = fixture();
    let t = &g.t;
    // churn so that the planner has statistics for both families
    t.tx(|tx| {
        for i in 0..200 {
            let e = tx
                .assert(
                    v(&format!("s{}", i % 20)),
                    v(&format!("p{}", i % 5)),
                    v(&format!("o{i}")),
                    Valid::ALWAYS,
                )?
                .eid();
            tx.add_to_graph(e, gv(&format!("{}", i % 3)), AssertOpts::default())?;
        }
        Ok(())
    });
    let graphs = [t.id(&gv("1")), t.id(&gv("2"))];
    let mut ex = exec_of(t);
    ex.connection().execute_batch("ANALYZE").unwrap();
    let views = [
        ("now", ResolvedView::NOW),
        (
            "asof",
            ResolvedView {
                tx: ResolvedTx::AsOf(1),
                valid: ValidSel::Unfiltered,
            },
        ),
        (
            "history",
            ResolvedView {
                tx: ResolvedTx::History,
                valid: ValidSel::Unfiltered,
            },
        ),
    ];
    let mut report = Vec::new();
    let mut texts = Vec::new();
    for (vname, view) in views {
        for (lname, l) in letters() {
            let sql = {
                let mut f = Fetcher::new(&mut ex, view)
                    .with_graphs(Some(&graphs))
                    .unwrap();
                f.sql_of(&l).unwrap()
            };
            assert!(sql.contains("EXISTS (SELECT 1 FROM triple AS m WHERE m.s = t.eid"));
            assert!(!sql.contains('\''), "no literal in the SQL: {sql}");
            for id in graphs {
                assert!(
                    !sql.contains(&id.raw().to_string()),
                    "graph ids are parameters"
                );
            }
            if vname == "now" {
                texts.push(format!("{lname}: {sql}"));
            }
            let plan: Vec<String> = {
                use rusqlite::types::Value as RV;
                use rusqlite::vtab::array::Array;
                let mut st = ex
                    .connection()
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                    .unwrap();
                let n = st.parameter_count();
                let arr = || -> Array { std::rc::Rc::new(vec![RV::Integer(1)]) };
                let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
                for i in 1..=n {
                    // the frontier array is ?1; the graph array is the other array
                    if sql.contains(&format!("rarray(?{i})")) {
                        vals.push(Box::new(arr()));
                    } else {
                        vals.push(Box::new(1i64));
                    }
                }
                let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
                st.query_map(refs.as_slice(), |r| r.get::<_, String>(3))
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect()
            };
            let line = format!("{vname} / {lname}: {}", plan.join(" | "));
            // the membership is a covering-index seek on the statement's eid (SQLite
            // may lead with the listed graph: `live_osp (o=? AND s=? AND p=? …)`)
            let seek = line
                .split(" | ")
                .find(|s| s.starts_with("SEARCH m"))
                .unwrap_or_else(|| panic!("no seek of m: {line}"));
            assert!(
                seek.contains("COVERING INDEX") && seek.contains("s=?") && seek.contains("p=?"),
                "{line}"
            );
            assert!(
                !line.contains("SCAN t ") && !line.contains("SCAN m"),
                "{line}"
            );
            report.push(line);
        }
    }
    insta::assert_snapshot!("graph_filter_sql", texts.join("\n"));
    insta::assert_snapshot!("graph_filter_plans", report.join("\n"));
}

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

// @lat: [[tests#Named Graphs#tm_path Graphs Argument]]
#[test]
fn tm_path_graphs_argument() {
    let g = fixture();
    let a = g.id("a").raw();
    let (g1, g2) = (g.t.id(&gv("1")).raw(), g.t.id(&gv("2")).raw());
    let names = |rows: &[Vec<SqlValue>], col: usize| -> Vec<String> {
        let mut v: Vec<String> = rows
            .iter()
            .map(|r| g.name(ObjectId::from_raw(r[col].as_i64().unwrap())))
            .collect();
        v.sort();
        v
    };
    // one graph as an integer, several as JSON text, NULL as no filter
    let one = q(
        &g,
        &format!("SELECT \"end\" FROM tm_path({a}, 'knows+', 'REACH', NULL, NULL, {g1})"),
    );
    assert_eq!(names(&one, 0), ["b", "c"]);
    let two = q(
        &g,
        &format!("SELECT \"end\" FROM tm_path({a}, 'knows+', 'REACH', NULL, NULL, '[{g1}, {g2}]')"),
    );
    assert_eq!(names(&two, 0), ["b", "c", "d"]);
    let all = q(
        &g,
        &format!("SELECT \"end\" FROM tm_path({a}, 'knows+', 'REACH', NULL, NULL, NULL)"),
    );
    assert_eq!(all.len(), 4);
    let empty = q(
        &g,
        &format!("SELECT hops FROM tm_path({a}, 'knows*', 'REACH', NULL, NULL, '[]')"),
    );
    assert_eq!(empty.len(), 1, "only the zero-hop row");
    // correlated with a column: one call per graph
    let inside = q(
        &g,
        &format!(
            "SELECT m.o, p.\"end\" FROM (SELECT DISTINCT o FROM triple WHERE p = {}) m, \
             tm_path({a}, 'knows+', 'REACH', NULL, NULL, m.o) p",
            g.t.id(&Value::iri(tiramemsu::vocab::SYS_IN_GRAPH)).raw()
        ),
    );
    let mut pairs: Vec<(Value, String)> = inside
        .iter()
        .map(|r| {
            let graph = ObjectId::from_raw(r[0].as_i64().unwrap());
            (
                g.t.db.now().decode(graph).unwrap(),
                g.name(ObjectId::from_raw(r[1].as_i64().unwrap())),
            )
        })
        .collect();
    pairs.sort_by(|x, y| x.1.cmp(&y.1));
    assert_eq!(
        pairs,
        vec![(gv("1"), "b".to_string()), (gv("1"), "c".to_string())],
        "g2 holds only `c knows d`, which `a` cannot reach inside g2"
    );
    // the pushed-down end still works after the sixth argument
    let pushed = q(
        &g,
        &format!(
            "SELECT \"end\" FROM tm_path({a}, 'knows+', 'REACH', NULL, NULL, {g1}) WHERE \"end\" = {}",
            g.id("c").raw()
        ),
    );
    assert_eq!(names(&pushed, 0), ["c"]);
    // malformed graphs
    for bad in ["'[1, \"g\"]'", "'g1'", "1.5", "'[1'", "x'00'"] {
        let m = err(
            &g,
            &format!("SELECT * FROM tm_path({a}, 'knows', 'REACH', NULL, NULL, {bad})"),
        );
        assert!(m.starts_with("tm_path: graphs:"), "{bad}: {m}");
    }
}

fn b() -> IrBuilder {
    IrBuilder::sparql()
}

fn gt(name: &str) -> TermOrVar {
    TermOrVar::iri(format!("urn:tiramemsu:g:{name}"))
}

fn path_in(start: &str, end: &str, sel: GraphSel) -> Op {
    match b().path(start, PathExpr::iri(vi("knows")).plus(), end, REACH) {
        Op::Path(p) => Op::Path(p.in_graph(sel)),
        _ => unreachable!("a path"),
    }
}

fn snap(g: &G, name: &str, q: &IrQuery) -> QueryResult {
    let ex = explain(&g.t.db.now(), q);
    let sql = ex.sql.unwrap_or_else(|| "-- short-circuit".to_string());
    let text = format!("{}\n-- params: {:?}", sql, ex.params.len());
    insta::with_settings!({ snapshot_suffix => name, prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!("path_graph_selector", text);
    });
    run(&g.t.db.now(), q)
}

// @lat: [[tests#Named Graphs#Path Graph Selector SQL Snapshots]]
#[test]
fn path_graph_selector_sql_snapshots() {
    let g = fixture();
    // a set of one graph: an integer argument
    let one = snap(
        &g,
        "set_one",
        &b().query(path_in("v:a", "?x", GraphSel::Set(vec![gt("1")])).project(&["x"])),
    );
    assert_eq!(sorted(&one), expect(&[&["b"], &["c"]]));
    // several graphs (one unknown): a JSON text argument
    let several = snap(
        &g,
        "set_several",
        &b().query(
            path_in(
                "v:a",
                "?x",
                GraphSel::Set(vec![gt("1"), gt("2"), gt("nope")]),
            )
            .project(&["x"]),
        ),
    );
    assert_eq!(sorted(&several), expect(&[&["b"], &["c"], &["d"]]));
    // a variable no other pattern binds: the graphs of the view are enumerated
    let enumerated = snap(
        &g,
        "var_enumerated",
        &b().query(path_in("v:a", "?x", GraphSel::Var("g".into())).project(&["g", "x"])),
    );
    assert_eq!(
        sorted(&enumerated),
        expect(&[&["<urn:tiramemsu:g:1>", "b"], &["<urn:tiramemsu:g:1>", "c"],])
    );
    // a variable bound by a triple pattern of the join: correlated
    let bound = Op::join(vec![
        Op::Triple(
            b().t("?s", "v:knows", "v:d")
                .in_graph(GraphSel::Var("g".into())),
        ),
        path_in("?s", "?x", GraphSel::Var("g".into())),
    ]);
    let correlated = snap(
        &g,
        "var_correlated",
        &b().query(bound.project(&["g", "s", "x"])),
    );
    assert_eq!(
        sorted(&correlated),
        expect(&[&["<urn:tiramemsu:g:2>", "c", "d"]])
    );
    // a parameter in the set is bound before planning
    let p = b()
        .query(path_in("v:a", "?x", GraphSel::Set(vec![TermOrVar::param("gr")])).project(&["x"]));
    let mut params = Params::new();
    params.insert("gr".to_string(), gv("2"));
    let r = g.t.db.now().execute_ir(&p, &params).unwrap();
    assert!(r.rows.is_empty(), "a knows nothing inside g2");
    // no `graphs` argument without a selector (existing plans are unchanged)
    let plain = explain(
        &g.t.db.now(),
        &b().query(path_in("v:a", "?x", GraphSel::Any).project(&["x"])),
    );
    let sql = plain.sql.unwrap();
    assert!(sql.contains("tm_path(?1, ?2, ?3, ?4, ?5) AS p0"), "{sql}");
}
