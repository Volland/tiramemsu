//! `cypher-read`: property access, isomorphism, WHERE, OPTIONAL MATCH, WITH, RETURN.
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

fn alice() -> CypherParams {
    params(&[("alice", node_ref(v("alice")))])
}

// "Missing property is null" / "Multi-valued property becomes a list" / "keys and properties"
#[test]
fn property_access() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("age"), Value::Int(42)),
    ]);
    let r = t.qp("MATCH (n) WHERE n = $alice RETURN n.email AS e", &alice());
    assert_eq!(r.rows, vec![vec![null()]]);
    let r = t.qp(
        "MATCH (n) WHERE n = $alice RETURN keys(n) AS k, properties(n) AS p",
        &alice(),
    );
    assert_eq!(r.rows[0][0], list(vec![s("age"), s("name")]));
    assert_eq!(r.rows[0][1], map(&[("age", i(42)), ("name", s("Alice"))]));
    t.assert(&[(v("alice"), v("nick"), sv("al"))]);
    t.assert(&[(v("alice"), v("nick"), sv("ally"))]);
    let r = t.qp("MATCH (n) WHERE n = $alice RETURN n.nick AS n", &alice());
    assert_eq!(r.rows[0][0], list(vec![s("al"), s("ally")]));
}

fn knows_once() -> T {
    let t = T::new();
    t.assert(&[(v("a"), v("knows"), v("b"))]);
    t
}

// "Same relationship not reused within a clause" / "Comma-separated patterns share the constraint"
// / "Separate clauses may reuse a relationship" / "Explicit default mode" / REPEATABLE ELEMENTS
#[test]
fn relationship_isomorphism() {
    let t = knows_once();
    assert_eq!(
        t.one("MATCH (x)-[r1:knows]-(y)-[r2:knows]-(z) RETURN count(*) AS c"),
        i(0)
    );
    assert_eq!(
        t.one("MATCH (x)-[r1:knows]->(y), (p)-[r2:knows]->(q) RETURN count(*) AS c"),
        i(0)
    );
    assert_eq!(
        t.one("MATCH (x)-[r1:knows]->(y) MATCH (p)-[r2:knows]->(q) RETURN count(*) AS c"),
        i(1)
    );
    assert_eq!(
        t.one(
            "MATCH DIFFERENT RELATIONSHIPS (x)-[r1:knows]-(y)-[r2:knows]-(z) RETURN count(*) AS c"
        ),
        i(0)
    );
    assert_eq!(
        t.one("MATCH REPEATABLE ELEMENTS (x)-[r1:knows]-(y)-[r2:knows]-(z) RETURN count(*) AS c"),
        i(2)
    );
}

fn ppl() -> T {
    let t = T::new();
    t.assert(&[
        (v("alice"), rdf_type(), v("Person")),
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("age"), Value::Int(20)),
        (v("alfred"), rdf_type(), v("Person")),
        (v("alfred"), v("name"), sv("Alfred")),
        (v("bob"), rdf_type(), v("Person")),
        (v("bob"), v("name"), sv("Bob")),
    ]);
    t
}

// "Null comparison drops the row" / "IS NULL" / "String predicates"
#[test]
fn where_three_valued() {
    let t = ppl();
    assert_eq!(
        t.q("MATCH (n:Person) WHERE n.age > 30 RETURN n").rows.len(),
        0
    );
    // only alice (age 20) has an age: NOT (20 > 30); the null ones are dropped
    assert_eq!(
        t.q("MATCH (n:Person) WHERE NOT (n.age > 30) RETURN n")
            .rows
            .len(),
        1
    );
    assert_eq!(
        t.q("MATCH (n:Person) WHERE n.age IS NULL RETURN n")
            .rows
            .len(),
        2
    );
    let r =
        t.q("MATCH (n:Person) WHERE n.name STARTS WITH 'Al' AND n.name CONTAINS 'i' RETURN n.name");
    assert_eq!(r.rows, vec![vec![s("Alice")]]);
}

// "Three-valued connectives" / "IN with null"
#[test]
fn connectives_and_in() {
    let t = T::new();
    let r = t.q("RETURN null OR true AS a, null AND false AS b, null AND true AS c, NOT null AS d, null XOR true AS e");
    assert_eq!(r.rows[0], vec![b(true), b(false), null(), null(), null()]);
    let r = t.q("RETURN 2 IN [1, null] AS a, 1 IN [1, null] AS b, 3 IN [1, 2] AS c");
    assert_eq!(r.rows[0], vec![null(), b(true), b(false)]);
}

// "Missing optional part yields nulls" / "WHERE inside OPTIONAL MATCH keeps the outer row"
#[test]
fn optional_match() {
    let t = T::new();
    t.assert(&[
        (v("bob"), rdf_type(), v("Person")),
        (v("alice"), rdf_type(), v("Person")),
        (v("alice"), v("name"), sv("Alice")),
        (v("alice"), v("worksAt"), v("acme")),
        (v("acme"), v("name"), sv("Acme")),
    ]);
    let r = t.q(
        "MATCH (p:Person) OPTIONAL MATCH (p)-[r:worksAt]->(c) RETURN p, r, c ORDER BY elementId(p)",
    );
    assert_eq!(r.rows.len(), 2);
    assert_eq!(short(&r.rows[0][0]), "alice");
    assert_eq!(r.rows[1][1], null());
    assert_eq!(r.rows[1][2], null());
    let r = t.q("MATCH (p {name:'Alice'}) OPTIONAL MATCH (p)-[:worksAt]->(c) WHERE c.name = 'Globex' RETURN p, c");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][1], null());
}

fn acme_globex() -> T {
    let t = T::new();
    let mut tr = vec![];
    for (p, c) in [
        ("a1", "acme"),
        ("a2", "acme"),
        ("a3", "acme"),
        ("g1", "globex"),
    ] {
        tr.push((v(p), v("worksAt"), v(c)));
        tr.push((v(p), v("name"), sv(p)));
    }
    tr.push((v("acme"), v("name"), sv("Acme")));
    tr.push((v("globex"), v("name"), sv("Globex")));
    tr.push((v("acme"), rdf_type(), v("Company")));
    tr.push((v("globex"), rdf_type(), v("Company")));
    t.assert(&tr);
    t
}

// "Filter on an aggregate through WITH" / "Variable dropped by WITH" / "WITH DISTINCT"
#[test]
fn with_projection() {
    let t = acme_globex();
    let r = t.q("MATCH (p)-[:worksAt]->(c) WITH c, count(p) AS n WHERE n > 1 RETURN c.name, n");
    assert_eq!(r.rows, vec![vec![s("Acme"), i(3)]]);
    assert!(matches!(
        t.qerr("MATCH (p)-[:worksAt]->(c) WITH c RETURN p"),
        Error::Parse { .. }
    ));
    assert_eq!(
        t.one("MATCH (p)-[:worksAt]->(c) WITH DISTINCT c RETURN count(c) AS n"),
        i(2)
    );
}

// "Column naming" / "RETURN DISTINCT treats nulls as equal" / "RETURN star"
#[test]
fn return_projection() {
    let t = ppl();
    let r = t.q("MATCH (n:Person) RETURN n.name, n.age AS years");
    assert_eq!(r.columns, vec!["n.name", "years"]);
    let r = t.q("MATCH (n:Person) RETURN DISTINCT n.age AS a ORDER BY a");
    assert_eq!(r.rows, vec![vec![i(20)], vec![null()]]);
    let t = acme_globex();
    let r = t.q("MATCH (b)<-[r:worksAt]-(a) RETURN *");
    assert_eq!(r.columns, vec!["a", "b", "r"]);
    // empty result keeps the columns
    let r = t.q("MATCH (b)<-[r:nothing]-(a) RETURN *");
    assert_eq!(r.columns, vec!["a", "b", "r"]);
    assert!(r.rows.is_empty());
}

// "Numeric ordering across integer and float" / "Nulls last ascending, first descending"
// / "SKIP and LIMIT with parameters" / "Negative LIMIT fails"
#[test]
fn ordering_skip_limit() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("score"), Value::Int(10)),
        (v("b"), v("score"), Value::Double(9.5)),
        (v("c"), v("score"), Value::Int(100)),
    ]);
    let r = t.q("MATCH (n) WHERE n.score IS NOT NULL RETURN n.score ORDER BY n.score");
    assert_eq!(r.rows, vec![vec![f(9.5)], vec![i(10)], vec![i(100)]]);
    let t = T::new();
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("a"), v("age"), Value::Int(30)),
        (v("b"), rdf_type(), v("Person")),
        (v("c"), rdf_type(), v("Person")),
        (v("c"), v("age"), Value::Int(20)),
    ]);
    let r = t.q("MATCH (n:Person) RETURN n.age ORDER BY n.age DESC");
    assert_eq!(r.rows, vec![vec![null()], vec![i(30)], vec![i(20)]]);
    let r = t.q("MATCH (n:Person) RETURN n.age ORDER BY n.age");
    assert_eq!(r.rows, vec![vec![i(20)], vec![i(30)], vec![null()]]);
    let t = T::new();
    let tr: Vec<_> = (1..=10)
        .flat_map(|k| {
            [
                (v(&format!("p{k}")), rdf_type(), v("Person")),
                (v(&format!("p{k}")), v("name"), sv(&format!("n{k:02}"))),
            ]
        })
        .collect();
    t.assert(&tr);
    let r = t.qp(
        "MATCH (n:Person) RETURN n.name ORDER BY n.name SKIP $s LIMIT $l",
        &params(&[("s", i(2)), ("l", i(3))]),
    );
    assert_eq!(r.rows, vec![vec![s("n03")], vec![s("n04")], vec![s("n05")]]);
    assert!(matches!(
        t.qerr("MATCH (n) RETURN n LIMIT -1"),
        Error::Parse { .. }
    ));
}

// "Unwind a list" / "Unwind null and empty list" / "Unwind a parameter list into a match"
#[test]
fn unwind() {
    let t = T::new();
    let r = t.q("UNWIND [1, 2, 3] AS x RETURN x * 10 AS y");
    assert_eq!(r.rows, vec![vec![i(10)], vec![i(20)], vec![i(30)]]);
    assert!(t.q("UNWIND null AS x RETURN x").rows.is_empty());
    assert!(t.q("UNWIND [] AS x RETURN x").rows.is_empty());
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("a"), v("name"), sv("Alice")),
    ]);
    let r = t.qp(
        "UNWIND $names AS nm MATCH (p:Person {name: nm}) RETURN p.name",
        &params(&[("names", list(vec![s("Alice"), s("Zed")]))]),
    );
    assert_eq!(r.rows, vec![vec![s("Alice")]]);
}

// "Implicit grouping" / "Aggregates on empty input" / "count ignores null and DISTINCT removes duplicates"
// / "Aggregate in WHERE is rejected"
#[test]
fn aggregation() {
    let t = acme_globex();
    let r = t.q("MATCH (p)-[:worksAt]->(c) RETURN c.name AS company, count(p) AS staff, collect(p.name) AS names");
    assert_eq!(r.rows.len(), 2);
    for row in &r.rows {
        let CypherValue::List(l) = &row[2] else {
            panic!()
        };
        assert_eq!(row[1], i(l.len() as i64));
    }
    let r = t.q("MATCH (n:Unicorn) RETURN count(n) AS c, collect(n) AS l, sum(n.x) AS s");
    assert_eq!(r.rows, vec![vec![i(0), list(vec![]), null()]]);
    let t = T::new();
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("a"), v("age"), Value::Int(30)),
        (v("b"), rdf_type(), v("Person")),
        (v("b"), v("age"), Value::Int(30)),
        (v("c"), rdf_type(), v("Person")),
    ]);
    let r =
        t.q("MATCH (n:Person) RETURN count(n.age) AS a, count(DISTINCT n.age) AS b, count(*) AS c");
    assert_eq!(r.rows, vec![vec![i(2), i(1), i(3)]]);
    assert!(matches!(
        t.qerr("MATCH (n) WHERE count(n) > 1 RETURN n"),
        Error::Parse { .. }
    ));
}
