//! `query-provenance`: which stored statements produced each SPARQL solution.
mod differential;
mod sparql_common;
use proptest::prelude::*;
use sparql_common::*;
use tiramemsu::*;

fn on() -> SparqlOptions {
    SparqlOptions {
        provenance: true,
        ..Default::default()
    }
}

/// A SELECT with provenance on `view`.
fn prov_on(view: &View<'_>, q: &str) -> Solutions {
    match view
        .sparql_with(q, &on())
        .unwrap_or_else(|e| panic!("{q}: {e}"))
    {
        SparqlResult::Solutions(s) => s,
        other => panic!("not solutions: {other:?}"),
    }
}

/// Rows paired with their provenance, sorted (for queries without ORDER BY).
fn pairs(s: &Solutions) -> Vec<(Vec<Option<Value>>, Vec<Eid>)> {
    let mut out: Vec<_> = (0..s.rows.len())
        .map(|i| (s.rows[i].clone(), s.provenance(i).unwrap().to_vec()))
        .collect();
    out.sort_by_key(|p| format!("{p:?}"));
    out
}

fn sorted(rows: &[Vec<Option<Value>>]) -> Vec<String> {
    let mut v: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
    v.sort();
    v
}

/// Runs `q` with provenance on the current view, checks that the solutions are
/// those of the plain query (as a multiset, or in order with `ordered`), and
/// returns rows with provenance.
fn check_on(view: &View<'_>, q: &str, ordered: bool) -> Vec<(Vec<Option<Value>>, Vec<Eid>)> {
    let with = prov_on(view, q);
    let plain = sel_on(view, q);
    assert_eq!(with.vars, plain.vars, "{q}");
    if ordered {
        assert_eq!(with.rows, plain.rows, "{q}");
    } else {
        assert_eq!(sorted(&with.rows), sorted(&plain.rows), "{q}");
    }
    for i in 0..with.rows.len() {
        let p = with.provenance(i).unwrap();
        assert!(p.windows(2).all(|w| w[0] < w[1]), "unsorted {p:?}");
    }
    if ordered {
        (0..with.rows.len())
            .map(|i| (with.rows[i].clone(), with.provenance(i).unwrap().to_vec()))
            .collect()
    } else {
        pairs(&with)
    }
}

fn check(t: &T, q: &str) -> Vec<(Vec<Option<Value>>, Vec<Eid>)> {
    check_on(&t.db.now(), q, false)
}

/// Asserts `(s, p, o)` and returns its eid.
fn put(tx: &mut Tx<'_>, s: impl IntoObject, p: Value, o: impl IntoObject) -> Result<Eid> {
    Ok(tx.assert(s, p, o, Valid::ALWAYS)?.eid())
}

// query-provenance "Matched statements count": BGP, OPTIONAL, UNION, annotations
// and fixed-length paths
// @lat: [[tests#Query Provenance#Provenance Lists Matched Statements]]
#[test]
fn matched_statements_are_listed() {
    let t = T::new();
    let mut e = Vec::new();
    t.tx(|tx| {
        e.push(put(tx, v("alice"), v("worksAt"), v("acme"))?); // 0
        e.push(put(tx, v("acme"), v("locatedIn"), v("paris"))?); // 1
        e.push(put(tx, v("bob"), v("name"), s("Bob"))?); // 2
        e.push(put(tx, v("bob"), v("age"), int(41))?); // 3
        e.push(put(tx, v("carol"), v("name"), s("Carol"))?); // 4
        let src = put(tx, e[0], v("source"), v("crawler"))?; // 5
        e.push(src);
        Ok(())
    });
    // basic graph pattern: both statements of the join
    let got = check(
        &t,
        "SELECT ?c WHERE { v:alice v:worksAt ?o . ?o v:locatedIn ?c }",
    );
    assert_eq!(got, vec![(some(&[v("paris")]), vec![e[0], e[1]])]);
    // OPTIONAL: present adds its statement, absent adds nothing
    let got = check(
        &t,
        "SELECT ?p ?a WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } }",
    );
    assert_eq!(
        got,
        vec![
            (some(&[v("bob"), int(41)]), vec![e[2], e[3]]),
            (vec![Some(v("carol")), None], vec![e[4]]),
        ]
    );
    // UNION: the branch that produced the row
    let got = check(
        &t,
        "SELECT ?x WHERE { { v:alice v:worksAt ?x } UNION { v:acme v:locatedIn ?x } }",
    );
    assert_eq!(
        got,
        vec![
            (some(&[v("acme")]), vec![e[0]]),
            (some(&[v("paris")]), vec![e[1]]),
        ]
    );
    // annotation syntax: the annotated statement and the annotation triple
    let got = check(
        &t,
        "SELECT ?s WHERE { v:alice v:worksAt v:acme {| v:source ?s |} }",
    );
    assert_eq!(got, vec![(some(&[v("crawler")]), vec![e[0], e[5]])]);
    // a reifier binds the same statement
    let got = check(
        &t,
        "SELECT ?r ?s WHERE { v:alice v:worksAt v:acme ~ ?r . ?r v:source ?s }",
    );
    assert_eq!(got[0].1, vec![e[0], e[5]]);
    // a fixed-length path lists each hop's statement
    let got = check(&t, "SELECT ?c WHERE { v:alice v:worksAt/v:locatedIn ?c }");
    assert_eq!(got, vec![(some(&[v("paris")]), vec![e[0], e[1]])]);
    // SELECT * keeps hidden columns out of the variables
    let s = prov_on(&t.db.now(), "SELECT * WHERE { v:alice v:worksAt ?o }");
    assert_eq!(s.vars, ["o"]);
}

// query-provenance "Matched statements count": tests, virtual predicates and
// recursive paths add nothing
// @lat: [[tests#Query Provenance#Tested Statements Are Not Cited]]
#[test]
fn tested_statements_are_not_cited() {
    let t = T::new();
    let mut e = Vec::new();
    t.tx(|tx| {
        e.push(put(tx, v("a"), v("p"), v("x"))?);
        e.push(put(tx, v("x"), v("q"), v("y"))?);
        e.push(put(tx, v("a"), v("p"), v("z"))?);
        e.push(put(tx, v("a"), v("knows"), v("b"))?);
        e.push(put(tx, v("b"), v("knows"), v("c"))?);
        Ok(())
    });
    let got = check(
        &t,
        "SELECT ?o WHERE { v:a v:p ?o FILTER EXISTS { ?o v:q ?w } }",
    );
    assert_eq!(got, vec![(some(&[v("x")]), vec![e[0]])]);
    let got = check(&t, "SELECT ?o WHERE { v:a v:p ?o MINUS { ?o v:q ?w } }");
    assert_eq!(got, vec![(some(&[v("z")]), vec![e[2]])]);
    let got = check(
        &t,
        "SELECT ?o WHERE { v:a v:p ?o FILTER NOT EXISTS { ?o v:q ?w } }",
    );
    assert_eq!(got, vec![(some(&[v("z")]), vec![e[2]])]);
    // a virtual predicate reads the statement's row and adds no eid
    let got = check(
        &t,
        "SELECT ?t WHERE { v:a v:p v:x ~ ?r . ?r tm:txAdded ?t }",
    );
    assert_eq!(got[0].1, vec![e[0]]);
    // a recursive path region carries no eids (documented limit)
    let got = check(&t, "SELECT ?x WHERE { v:a v:knows+ ?x }");
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|(_, p)| p.is_empty()));
    let got = check(&t, "SELECT ?x WHERE { v:a v:knows+ ?x . ?x v:knows ?y }");
    assert_eq!(got, vec![(some(&[v("b")]), vec![e[4]])]);
}

// add-mcp-adapter "Incomplete provenance": a recursive path is a gap the
// solutions report, while fixed-length paths and text matches are cited
// @lat: [[tests#Query Provenance#Provenance Reports Its Gaps]]
#[test]
fn provenance_reports_its_gaps() {
    let t = T::new();
    let mut e = Vec::new();
    t.tx(|tx| {
        e.push(put(tx, v("a"), v("knows"), v("b"))?);
        e.push(put(tx, v("b"), v("knows"), v("c"))?);
        e.push(put(tx, v("a"), v("note"), Value::str("lisbon offsite"))?);
        Ok(())
    });
    let gaps = |q: &str| {
        let s = prov_on(&t.db.now(), q);
        (s.provenance_gaps.clone(), s.provenance_complete())
    };
    assert_eq!(
        gaps("SELECT ?x WHERE { v:a v:knows+ ?x }"),
        (vec![ProvenanceGap::RecursivePath], Some(false))
    );
    assert_eq!(
        gaps("SELECT ?x WHERE { v:a v:knows* ?x . ?x v:knows ?y }"),
        (vec![ProvenanceGap::RecursivePath], Some(false))
    );
    assert_eq!(
        gaps("SELECT ?x WHERE { v:a v:knows/v:knows ?x }"),
        (vec![], Some(true))
    );
    // without provenance there is no verdict
    let plain = t.sel("SELECT ?x WHERE { v:a v:knows+ ?x }");
    assert_eq!(plain.provenance_complete(), None);
    assert!(plain.provenance_gaps.is_empty());
    // a text match cites the statement it matched
    t.db.enable_text_index().unwrap();
    let s = prov_on(
        &t.db.now(),
        "SELECT ?e WHERE { ?e tm:textMatch \"lisbon\" }",
    );
    assert_eq!(s.provenance(0), Some(&[e[2]][..]));
    assert_eq!(s.provenance_complete(), Some(true));
}

// add-mcp-adapter "Read-only mutation": query-only text cannot write
// @lat: [[tests#Query Provenance#Query Only Refuses Updates]]
#[test]
fn query_only_refuses_updates() {
    let t = T::new();
    let read = SparqlOptions {
        query_only: true,
        ..Default::default()
    };
    for q in [
        "INSERT DATA { v:a v:p v:b }",
        "DELETE WHERE { ?s ?p ?o }",
        "CLEAR GRAPH <urn:g>",
    ] {
        let r = t.db.now().sparql_with(q, &read);
        assert!(
            matches!(&r, Err(Error::Unsupported { feature }) if feature == "update in a query-only call"),
            "{q}: {r:?}"
        );
    }
    assert!(t.live().is_empty());
    assert_eq!(t.last_t(), 0);
    // queries of every form still run
    t.upd("INSERT DATA { v:a v:p v:b }");
    assert!(t.db.now().sparql_with("ASK { v:a v:p v:b }", &read).is_ok());
    assert!(t
        .db
        .now()
        .sparql_with("SELECT ?o WHERE { v:a v:p ?o }", &read)
        .is_ok());
}

// query-provenance "Modifiers, aggregates and subqueries": DISTINCT, LIMIT,
// OFFSET and ORDER BY
// @lat: [[tests#Query Provenance#Distinct Merges Provenance]]
#[test]
fn distinct_merges_and_slices_stay_exact() {
    let t = T::new();
    let mut e = Vec::new();
    t.tx(|tx| {
        for (a, b) in [
            ("alice", "bob"),
            ("alice", "carol"),
            ("bob", "dan"),
            ("bob", "erin"),
            ("carl", "fay"),
        ] {
            e.push(put(tx, v(a), v("knows"), v(b))?);
        }
        Ok(())
    });
    let now = t.db.now();
    let got = check(&t, "SELECT DISTINCT ?s WHERE { ?s v:knows ?o }");
    assert_eq!(
        got,
        vec![
            (some(&[v("alice")]), vec![e[0], e[1]]),
            (some(&[v("bob")]), vec![e[2], e[3]]),
            (some(&[v("carl")]), vec![e[4]]),
        ]
    );
    // the slice counts merged rows, in ORDER BY order
    let got = check_on(
        &now,
        "SELECT DISTINCT ?s WHERE { ?s v:knows ?o } ORDER BY ?s LIMIT 1",
        true,
    );
    assert_eq!(got, vec![(some(&[v("alice")]), vec![e[0], e[1]])]);
    let got = check_on(
        &now,
        "SELECT DISTINCT ?s WHERE { ?s v:knows ?o } ORDER BY DESC(?s) OFFSET 1 LIMIT 5",
        true,
    );
    assert_eq!(
        got,
        vec![
            (some(&[v("bob")]), vec![e[2], e[3]]),
            (some(&[v("alice")]), vec![e[0], e[1]]),
        ]
    );
    // without DISTINCT the slice stays in SQL and each row keeps its own eid
    let got = check_on(
        &now,
        "SELECT ?s ?o WHERE { ?s v:knows ?o } ORDER BY ?o LIMIT 2",
        true,
    );
    assert_eq!(
        got,
        vec![
            (some(&[v("alice"), v("bob")]), vec![e[0]]),
            (some(&[v("alice"), v("carol")]), vec![e[1]]),
        ]
    );
    // REDUCED keeps duplicates, each with its own provenance
    let got = check(&t, "SELECT REDUCED ?s WHERE { ?s v:knows ?o }");
    assert_eq!(got.len(), 5);
}

// query-provenance "Modifiers, aggregates and subqueries": groups and subqueries
// @lat: [[tests#Query Provenance#Groups And Subqueries Union Provenance]]
#[test]
fn groups_and_subqueries_union_provenance() {
    let t = T::new();
    let mut e = Vec::new();
    t.tx(|tx| {
        e.push(put(tx, v("alice"), v("knows"), v("bob"))?); // 0
        e.push(put(tx, v("alice"), v("knows"), v("carol"))?); // 1
        e.push(put(tx, v("bob"), v("knows"), v("dan"))?); // 2
        e.push(put(tx, v("alice"), v("worksAt"), v("acme"))?); // 3
        e.push(put(tx, v("bob"), v("worksAt"), v("initech"))?); // 4
        Ok(())
    });
    let got = check(
        &t,
        "SELECT ?s (COUNT(*) AS ?n) WHERE { ?s v:knows ?o } GROUP BY ?s",
    );
    assert_eq!(
        got,
        vec![
            (some(&[v("alice"), int(2)]), vec![e[0], e[1]]),
            (some(&[v("bob"), int(1)]), vec![e[2]]),
        ]
    );
    // HAVING filters groups; the kept group keeps its provenance
    let got = check(
        &t,
        "SELECT ?s WHERE { ?s v:knows ?o } GROUP BY ?s HAVING (COUNT(*) > 1)",
    );
    assert_eq!(got, vec![(some(&[v("alice")]), vec![e[0], e[1]])]);
    // one group over no rows: an empty list
    let got = check(&t, "SELECT (COUNT(*) AS ?n) WHERE { ?s v:hates ?o }");
    assert_eq!(got, vec![(some(&[int(0)]), vec![])]);
    // a plain subquery carries its rows' provenance into the join
    let got = check(
        &t,
        "SELECT ?s ?c WHERE { ?s v:worksAt ?c { SELECT ?s WHERE { ?s v:knows v:dan } } }",
    );
    assert_eq!(
        got,
        vec![(some(&[v("bob"), v("initech")]), vec![e[2], e[4]])]
    );
    // a DISTINCT subquery unions the rows it merges
    let got = check(
        &t,
        "SELECT ?s ?c WHERE { ?s v:worksAt ?c { SELECT DISTINCT ?s WHERE { ?s v:knows ?o } } }",
    );
    assert_eq!(
        got,
        vec![
            (some(&[v("alice"), v("acme")]), vec![e[0], e[1], e[3]]),
            (some(&[v("bob"), v("initech")]), vec![e[2], e[4]]),
        ]
    );
    // a grouped subquery inside a group: lists nest
    let got = check(
        &t,
        "SELECT (SUM(?k) AS ?n) WHERE { { SELECT ?s (COUNT(*) AS ?k) WHERE { ?s v:knows ?o } GROUP BY ?s } }",
    );
    assert_eq!(got, vec![(some(&[int(3)]), vec![e[0], e[1], e[2]])]);
    // a reifier inside a subquery is not exposed to the outer query
    let got = check(
        &t,
        "SELECT ?s ?r WHERE { ?s v:worksAt v:acme { SELECT ?s WHERE { ?s v:knows v:bob ~ ?r } } }",
    );
    assert_eq!(got, vec![(vec![Some(v("alice")), None], vec![e[0], e[3]])]);
}

// query-provenance "One SPARQL triple lists all its eids"
// @lat: [[tests#Query Provenance#One Triple Lists All Its Eids]]
#[test]
fn one_triple_lists_all_its_eids() {
    let t = T::new();
    let mut e = Vec::new();
    t.tx(|tx| {
        for (a, b) in [(0, 1_000), (2_000, 3_000)] {
            e.push(
                tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::between(a, b))?
                    .eid(),
            );
        }
        e.push(tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?);
        e.push(tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?);
        Ok(())
    });
    let got = check(&t, "SELECT ?c WHERE { v:alice v:worksAt ?c }");
    assert_eq!(got, vec![(some(&[v("acme")]), vec![e[0], e[1]])]);
    let got = check(&t, "SELECT ?p ?o WHERE { v:alice ?p ?o }");
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].1, vec![e[2], e[3]]);
    // a reifier still gives one row per eid, each with its own
    let got = check(&t, "SELECT ?r WHERE { v:alice v:called v:bob ~ ?r }");
    assert_eq!(
        got.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>(),
        vec![vec![e[2]], vec![e[3]]]
    );
    // a valid-time view sees one episode: only its eid
    let got = check_on(
        &t.db.now().valid_at(500),
        "SELECT ?c WHERE { v:alice v:worksAt ?c }",
        false,
    );
    assert_eq!(got, vec![(some(&[v("acme")]), vec![e[0]])]);
    // after one parallel edge is retracted, only the live one is cited
    t.tx(|tx| {
        tx.retract(e[2])?;
        Ok(())
    });
    let got = check(&t, "SELECT ?o WHERE { v:alice v:called ?o }");
    assert_eq!(got, vec![(some(&[v("bob")]), vec![e[3]])]);
}

// query-provenance "Time scopes and graphs": GRAPH memberships and FROM graphs
// @lat: [[tests#Query Provenance#Graphs Cite Memberships]]
#[test]
fn graphs_cite_memberships() {
    let t = T::new();
    let (mut e, mut m) = (Vec::new(), Vec::new());
    t.tx(|tx| {
        e.push(put(tx, v("alice"), v("worksAt"), v("acme"))?);
        e.push(put(tx, v("bob"), v("worksAt"), v("initech"))?);
        m.push(tx.add_to_graph(e[0], v("g1"), AssertOpts::default())?.0);
        m.push(tx.add_to_graph(e[0], v("g2"), AssertOpts::default())?.0);
        m.push(tx.add_to_graph(e[1], v("g2"), AssertOpts::default())?.0);
        Ok(())
    });
    let got = check(
        &t,
        "SELECT ?c WHERE { GRAPH v:g1 { v:alice v:worksAt ?c } }",
    );
    assert_eq!(got, vec![(some(&[v("acme")]), vec![e[0], m[0]])]);
    // GRAPH ?g: one row per membership, each citing its own
    let got = check(&t, "SELECT ?g ?c WHERE { GRAPH ?g { ?s v:worksAt ?c } }");
    assert_eq!(
        got,
        vec![
            (some(&[v("g1"), v("acme")]), vec![e[0], m[0]]),
            (some(&[v("g2"), v("acme")]), vec![e[0], m[1]]),
            (some(&[v("g2"), v("initech")]), vec![e[1], m[2]]),
        ]
    );
    // FROM of one graph is a membership join too
    let got = check(&t, "SELECT ?c FROM v:g2 WHERE { v:bob v:worksAt ?c }");
    assert_eq!(got, vec![(some(&[v("initech")]), vec![e[1], m[2]])]);
    // several FROM graphs test membership with EXISTS: the statement only
    let got = check(
        &t,
        "SELECT ?c FROM v:g1 FROM v:g2 WHERE { v:alice v:worksAt ?c }",
    );
    assert_eq!(got, vec![(some(&[v("acme")]), vec![e[0]])]);
    // a membership read as an ordinary statement is cited like any other
    let got = check(&t, "SELECT ?m WHERE { ?e sys:inGraph v:g1 ~ ?m }");
    assert_eq!(got, vec![(vec![Some(Value::Stmt(m[0]))], vec![m[0]])]);
}

// query-provenance "Time scopes and graphs": a SERVICE scope cites past statements
// @lat: [[tests#Query Provenance#Time Scopes Cite Past Statements]]
#[test]
fn time_scopes_cite_past_statements() {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        e1 = Some(put(tx, v("alice"), v("worksAt"), v("acme"))?);
        Ok(())
    });
    let e1 = e1.unwrap();
    let t1 = t.last_t();
    let mut e2 = None;
    t.tx(|tx| {
        tx.retract(e1)?;
        e2 = Some(put(tx, v("alice"), v("worksAt"), v("globex"))?);
        Ok(())
    });
    let e2 = e2.unwrap();
    let q = format!(
        "SELECT ?old ?new WHERE {{ SERVICE <urn:tiramemsu:tm:asOf/{t1}> {{ v:alice v:worksAt ?old }} \
         v:alice v:worksAt ?new }}"
    );
    let got = check(&t, &q);
    assert_eq!(got, vec![(some(&[v("acme"), v("globex")]), vec![e1, e2])]);
    // the cited e1 is retracted now: invisible in the current view
    let live: Vec<Eid> =
        t.db.now()
            .triples(None, None, None)
            .unwrap()
            .iter()
            .map(|x| x.eid)
            .collect();
    assert!(!live.contains(&e1) && live.contains(&e2));
    // history sees both episodes of the query's one pattern
    let got = check_on(
        &t.db.history(),
        "SELECT ?c WHERE { v:alice v:worksAt ?c }",
        false,
    );
    assert_eq!(
        got,
        vec![
            (some(&[v("acme")]), vec![e1]),
            (some(&[v("globex")]), vec![e2])
        ]
    );
}

// query-provenance "Provenance is requested per query" and "Unsupported
// combinations fail before execution"
// @lat: [[tests#Query Provenance#Provenance Is Off By Default]]
#[test]
fn provenance_is_off_by_default() {
    let t = T::new();
    t.assert(&[
        (v("bob"), v("name"), s("Bob")),
        (v("bob"), v("age"), int(41)),
    ]);
    let q = "SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age } }";
    let now = t.db.now();
    let plain = now.sparql(q).unwrap();
    let off = now.sparql_with(q, &SparqlOptions::default()).unwrap();
    assert_eq!(plain, off);
    assert_eq!(plain.solutions().unwrap().provenance(0), None);
    let doc = plain.write_sparql_json().unwrap();
    assert_eq!(doc, off.write_sparql_json().unwrap());
    assert!(!doc.contains("provenance"));
    // with provenance the standard members are unchanged and one is added
    let with = now
        .sparql_with(q, &on())
        .unwrap()
        .write_sparql_json()
        .unwrap();
    let (start, end) = (
        with.find(r#""provenance":"#).unwrap(),
        with.find(r#""results":"#).unwrap(),
    );
    assert_eq!(format!("{}{}", &with[..start], &with[end..]), doc);
    assert!(
        with[start..end].starts_with(r#""provenance":[["urn:tiramemsu:stmt:"#),
        "{with}"
    );
    // ASK and CONSTRUCT have no rows to annotate; updates write nothing
    for (q, want) in [
        ("ASK { ?s ?p ?o }", "provenance for ASK"),
        (
            "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
            "provenance for CONSTRUCT",
        ),
        ("INSERT DATA { v:a v:p v:b }", "provenance for updates"),
    ] {
        let err = now.sparql_with(q, &on()).unwrap_err();
        assert!(
            matches!(&err, Error::Unsupported { feature } if feature == want),
            "{q}: {err:?}"
        );
    }
    assert!(!t.has(&v("a"), &v("p"), &v("b")));
}

// query-provenance "Stale answers can be detected"
// @lat: [[tests#Query Provenance#Stale Answers Are Detectable]]
#[test]
fn stale_answers_are_detectable() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("worksAt"), v("acme")),
        (v("acme"), v("locatedIn"), v("paris")),
    ]);
    // an agent answers "where does alice work?" and keeps the citations
    let q = "SELECT ?c WHERE { v:alice v:worksAt ?o . ?o v:locatedIn ?c }";
    let answer = prov_on(&t.db.now(), q);
    let cited = answer.provenance(0).unwrap().to_vec();
    assert_eq!(cited.len(), 2);
    // later, acme moves
    let report = t.retract(&v("acme"), &v("locatedIn"), &v("paris"));
    // which cited statements are no longer live?
    let live: Vec<Eid> =
        t.db.now()
            .triples(None, None, None)
            .unwrap()
            .iter()
            .map(|x| x.eid)
            .collect();
    let stale: Vec<Eid> = cited
        .iter()
        .filter(|e| !live.contains(e))
        .copied()
        .collect();
    assert_eq!(stale, vec![cited[1]]);
    // the same check in SPARQL, with the retracting transaction
    let values: Vec<String> = cited
        .iter()
        .map(|e| format!("<urn:tiramemsu:stmt:{}>", e.n()))
        .collect();
    let s = sel_on(
        &t.db.history(),
        &format!(
            "SELECT ?e ?t WHERE {{ VALUES ?e {{ {} }} ?e tm:txRetracted ?t }}",
            values.join(" ")
        ),
    );
    assert_eq!(s.rows.len(), 1);
    assert_eq!(s.get(0, "e"), Some(&Value::Stmt(cited[1])));
    assert_eq!(s.get(0, "t"), Some(&Value::Tx(report.t)));
}

/// The view of a differential corpus pair.
fn corpus_view<'a>(db: &'a Db, spec: &Option<String>) -> View<'a> {
    match spec.as_deref() {
        None | Some("now") => db.now(),
        Some("history") => db.history(),
        Some(s) if s.starts_with("asof:") => db.as_of(TimeRef::Tx(s[5..].parse().unwrap())),
        Some(s) if s.starts_with("asof-instant:") => {
            db.as_of(TimeRef::Instant(s[13..].parse().unwrap()))
        }
        Some(s) if s.starts_with("valid:") => db.now().valid_at(s[6..].parse().unwrap()),
        Some(other) => panic!("unknown view {other}"),
    }
}

// query-provenance "Provenance is requested per query": Solutions do not change,
// over every SELECT of the differential corpus
// @lat: [[tests#Query Provenance#Provenance Keeps The Solutions]]
#[test]
fn corpus_solutions_do_not_change() {
    let mut fixtures: std::collections::HashMap<String, differential::fixtures::Fx> =
        std::collections::HashMap::new();
    let mut ran = 0;
    for p in differential::runner::corpus() {
        let fx = fixtures
            .entry(p.fixture.clone())
            .or_insert_with(|| differential::fixtures::load(&p.fixture));
        let view = corpus_view(&fx.db, &p.view);
        if !matches!(view.sparql(&p.sparql), Ok(SparqlResult::Solutions(_))) {
            continue;
        }
        let rows = check_on(&view, &p.sparql, p.order != "unordered");
        if !p.sparql.contains("SERVICE") && !p.sparql.contains("FROM") {
            // every cited statement is visible in the view the query read (a time
            // clause reads another view)
            let visible: Vec<Eid> = view
                .triples(None, None, None)
                .unwrap()
                .iter()
                .map(|x| x.eid)
                .collect();
            for (_, cited) in &rows {
                assert!(
                    cited.iter().all(|e| visible.contains(e)),
                    "{}: {cited:?}",
                    p.name
                );
            }
        }
        ran += 1;
    }
    assert!(ran >= 40, "only {ran} corpus queries ran");
}

fn arb_data() -> impl Strategy<Value = Vec<(u8, u8, u8, bool)>> {
    prop::collection::vec((0u8..4, 0u8..3, 0u8..4, any::<bool>()), 1..12)
}

fn arb_bgp() -> impl Strategy<Value = Vec<(u8, u8, u8)>> {
    // positions: 0..4 a node constant, 4..7 a variable ?x ?y ?z; predicates 0..3
    prop::collection::vec((0u8..7, 0u8..3, 0u8..7), 1..4)
}

fn term(n: u8) -> String {
    match n {
        0..=3 => format!("v:n{n}"),
        4 => "?x".into(),
        5 => "?y".into(),
        _ => "?z".into(),
    }
}

fn bgp_text(bgp: &[(u8, u8, u8)]) -> String {
    let body: Vec<String> = bgp
        .iter()
        .map(|(s, p, o)| format!("{} v:p{p} {}", term(*s), term(*o)))
        .collect();
    format!("SELECT * WHERE {{ {} }}", body.join(" . "))
}

/// A fresh store holding copies of `triples`.
fn store(triples: &[(Value, Value, Value)]) -> T {
    let t = T::new();
    t.assert(triples);
    t
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    // query-provenance: provenance never changes the solutions, every cited eid is
    // visible in the view, and the cited statements alone reproduce the row
    // @lat: [[tests#Query Provenance#Provenance Is Sound]]
    #[test]
    fn provenance_is_sound(data in arb_data(), bgp in arb_bgp()) {
        let t = T::new();
        t.tx(|tx| {
            for (s, p, o, twice) in &data {
                let (s, p, o) = (v(&format!("n{s}")), v(&format!("p{p}")), v(&format!("n{o}")));
                tx.assert(s.clone(), p.clone(), o.clone(), Valid::ALWAYS)?;
                if *twice {
                    // a parallel statement with the same (s, p, o)
                    tx.create(s, p, o, Valid::ALWAYS)?;
                }
            }
            Ok(())
        });
        let q = bgp_text(&bgp);
        let now = t.db.now();
        let rows = check_on(&now, &q, false);
        let visible: Vec<Triple> = now.triples(None, None, None).unwrap();
        for (row, cited) in rows {
            let mut spo = Vec::new();
            for e in &cited {
                let tr = visible.iter().find(|x| x.eid == *e);
                prop_assert!(tr.is_some(), "cited {e:?} is not visible");
                let tr = tr.unwrap();
                spo.push((now.decode(tr.s).unwrap(), now.decode(tr.p).unwrap(), now.decode(tr.o).unwrap()));
            }
            let small = store(&spo);
            let again = sel_on(&small.db.now(), &q);
            prop_assert!(again.rows.contains(&row), "{q}: {row:?} not reproduced from {spo:?}");
        }
    }
}
