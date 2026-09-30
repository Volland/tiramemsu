//! `cypher-read`: subqueries, UNION, EXISTS, expressions, parameters, values, paths,
//! volatile properties, procedures, unsupported features and errors.
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

fn company_fixture() -> T {
    let t = T::new();
    let mut tr = vec![];
    for (p, c) in [("a1", "acme"), ("a2", "acme"), ("a3", "acme")] {
        tr.push((v(p), v("worksAt"), v(c)));
        tr.push((v(p), v("name"), sv(p)));
        tr.push((v(p), rdf_type(), v("Person")));
    }
    tr.push((v("acme"), rdf_type(), v("Company")));
    tr.push((v("acme"), v("name"), sv("Acme")));
    tr.push((v("initech"), rdf_type(), v("Company")));
    tr.push((v("initech"), v("name"), sv("Initech")));
    t.assert(&tr);
    t
}

// "Uncorrelated subquery is a cross product"
#[test]
fn uncorrelated_subquery_is_a_cross_product() {
    let t = T::new();
    t.assert(&[
        (v("p1"), rdf_type(), v("Person")),
        (v("p2"), rdf_type(), v("Person")),
        (v("c1"), rdf_type(), v("Company")),
        (v("c2"), rdf_type(), v("Company")),
        (v("c3"), rdf_type(), v("Company")),
    ]);
    assert_eq!(
        t.one("MATCH (p:Person) CALL { MATCH (c:Company) RETURN c } RETURN count(*) AS n"),
        i(6)
    );
}

// "Importing WITH with per-row aggregation" / "Empty correlated subquery drops the outer row"
#[test]
fn correlated_subquery() {
    let t = company_fixture();
    let r = t.q(
        "MATCH (c:Company) CALL { WITH c OPTIONAL MATCH (p)-[:worksAt]->(c) RETURN count(p) AS staff } \
         RETURN c.name, staff ORDER BY c.name",
    );
    assert_eq!(
        r.rows,
        vec![vec![s("Acme"), i(3)], vec![s("Initech"), i(0)]]
    );
    let r = t.q(
        "MATCH (c:Company) CALL { WITH c MATCH (p)-[:worksAt]->(c) RETURN p } RETURN c.name, p.name",
    );
    assert!(r.rows.iter().all(|x| x[0] != s("Initech")));
    assert_eq!(r.rows.len(), 3);
}

// "Shadowing an outer variable is rejected"
#[test]
fn subquery_shadowing_is_rejected() {
    let t = T::new();
    assert!(matches!(
        t.qerr("MATCH (p:Person) CALL { MATCH (p:Company) RETURN p } RETURN p"),
        Error::Parse { .. }
    ));
}

// "UNION removes duplicates" / "Column mismatch"
#[test]
fn union() {
    let t = T::new();
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("a"), v("name"), sv("A")),
        (v("b"), rdf_type(), v("Person")),
        (v("b"), v("name"), sv("B")),
    ]);
    let q = "MATCH (n:Person) RETURN n.name AS x UNION MATCH (n:Person) RETURN n.name AS x";
    assert_eq!(t.q(q).rows.len(), 2);
    assert_eq!(t.q(&q.replace("UNION", "UNION ALL")).rows.len(), 4);
    assert!(matches!(
        t.qerr("RETURN 1 AS a UNION RETURN 2 AS b"),
        Error::Parse { .. }
    ));
}

// "NOT EXISTS" / "Pattern predicate does not multiply rows"
#[test]
fn existential_subqueries() {
    let t = T::new();
    t.assert(&[
        (v("alice"), rdf_type(), v("Person")),
        (v("bob"), rdf_type(), v("Person")),
        (v("alice"), v("worksAt"), v("acme")),
        (v("alice"), v("worksAt"), v("globex")),
    ]);
    let r = t.q("MATCH (p:Person) WHERE NOT EXISTS { (p)-[:worksAt]->() } RETURN p");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "bob");
    let r = t.q("MATCH (p:Person) WHERE (p)-[:worksAt]->() RETURN p");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(short(&r.rows[0][0]), "alice");
    let r = t.q("MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:worksAt]->(c) WHERE c = c } RETURN p");
    assert_eq!(r.rows.len(), 1);
}

// "CASE and coalesce" / "List comprehension" / "Unknown function" / "Pattern comprehension unsupported"
#[test]
fn expressions() {
    let t = T::new();
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("a"), v("name"), sv("Ann")),
        (v("a"), v("age"), Value::Int(30)),
        (v("b"), rdf_type(), v("Person")),
        (v("b"), v("name"), sv("Bob")),
        (v("b"), v("age"), Value::Int(10)),
        (v("b"), v("nick"), sv("bobby")),
    ]);
    let r = t.q(
        "MATCH (n:Person) RETURN n.name, CASE WHEN n.age >= 18 THEN 'adult' ELSE 'minor' END AS k, \
         coalesce(n.nick, n.name) AS shown ORDER BY n.name",
    );
    assert_eq!(r.rows[0], vec![s("Ann"), s("adult"), s("Ann")]);
    assert_eq!(r.rows[1], vec![s("Bob"), s("minor"), s("bobby")]);
    assert_eq!(
        t.one("RETURN [x IN range(1, 5) WHERE x % 2 = 1 | x * x] AS l"),
        list(vec![i(1), i(9), i(25)])
    );
    assert!(
        matches!(t.qerr("RETURN apoc.text.clean('x')"), Error::Unsupported { ref feature } if feature.contains("apoc.text.clean"))
    );
    assert!(
        matches!(t.qerr("MATCH (a) RETURN [(a)-->(b) | b.name]"), Error::Unsupported { ref feature } if feature.contains("pattern comprehension"))
    );
}

#[test]
fn function_library() {
    let t = T::new();
    let r = t.q(
        "RETURN size([1,2,3]) AS a, head([4,5]) AS b, last([4,5]) AS c, toString(12) AS d, toInteger('7') AS e, \
         toFloat('1.5') AS f, toLower('AbC') AS g, toUpper('abc') AS h, trim('  x ') AS i, substring('hello', 1, 3) AS j, \
         replace('aXa', 'X', '-') AS k, split('a,b', ',') AS l, left('hello', 2) AS m, right('hello', 2) AS n, \
         reverse('abc') AS o, abs(-3) AS p, ceil(1.2) AS q, floor(1.8) AS r, round(1.5) AS s, sign(-9) AS t, sqrt(16) AS u",
    );
    assert_eq!(
        r.rows[0],
        vec![
            i(3),
            i(4),
            i(5),
            s("12"),
            i(7),
            f(1.5),
            s("abc"),
            s("ABC"),
            s("x"),
            s("ell"),
            s("a-a"),
            list(vec![s("a"), s("b")]),
            s("he"),
            s("lo"),
            s("cba"),
            i(3),
            f(2.0),
            f(1.0),
            f(2.0),
            i(-1),
            f(4.0)
        ]
    );
    assert_eq!(t.one("RETURN [1,2,3,4][1..3] AS x"), list(vec![i(2), i(3)]));
    assert_eq!(t.one("RETURN 'a' + 'b' AS x"), s("ab"));
    assert_eq!(t.one("RETURN {a: 1, b: [1, 2]}['b'][1] AS x"), i(2));
    assert_eq!(t.one("RETURN 'abc' =~ 'a.c' AS x"), b(true));
    assert_eq!(t.one("RETURN toInteger('x') AS v"), null());
}

// "Parameter in a property map" / "Missing parameter"
#[test]
fn parameters() {
    let t = T::new();
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("a"), v("name"), sv("Alice")),
    ]);
    let r = t.qp(
        "MATCH (n:Person {name: $who}) RETURN n",
        &params(&[("who", s("Alice"))]),
    );
    assert_eq!(r.rows.len(), 1);
    let e =
        t.db.now()
            .cypher("MATCH (n {name: $who}) RETURN n", &no_params())
            .unwrap_err();
    assert!(
        matches!(e, Error::Parse { ref msg, .. } if msg.contains("who")),
        "{e:?}"
    );
}

// "Scalar round trip of types"
#[test]
fn scalar_round_trip_of_types() {
    let t = T::new();
    t.assert(&[
        (v("x"), v("i"), Value::Int(7)),
        (v("x"), v("f"), Value::Double(1.5)),
        (v("x"), v("b"), Value::Bool(true)),
        (
            v("x"),
            v("d"),
            Value::literal("2025-03-01", Some(vocab::XSD_DATE), None),
        ),
        (
            v("x"),
            v("t"),
            Value::literal("2025-03-01T10:00:00+02:00", Some(vocab::XSD_DATETIME), None),
        ),
        (v("x"), v("s"), Value::literal("hé", None, Some("fr"))),
    ]);
    let r = t.qp(
        "MATCH (n) WHERE n = $x RETURN n.i, n.f, n.b, n.d, n.t, n.s",
        &params(&[("x", node_ref(v("x")))]),
    );
    let row = &r.rows[0];
    assert_eq!(row[0], i(7));
    assert_eq!(row[1], f(1.5));
    assert_eq!(row[2], b(true));
    assert!(matches!(row[3], CypherValue::Date(_)));
    assert!(matches!(row[4], CypherValue::DateTime { tz: 120, .. }));
    assert_eq!(row[5], s("hé"));
    let json = r.to_json();
    assert_eq!(json["rows"][0][4], "2025-03-01T10:00:00.000+02:00");
    assert_eq!(json["rows"][0][3], "2025-03-01");
}

// "DateTime equality compares instants" / "Date-time without timezone reads as LocalDateTime"
#[test]
fn datetime_equality_and_local() {
    let t = T::new();
    t.assert(&[
        (
            v("x"),
            v("a"),
            Value::literal("2026-03-01T12:00:00+02:00", Some(vocab::XSD_DATETIME), None),
        ),
        (
            v("x"),
            v("b"),
            Value::literal("2026-03-01T10:00:00Z", Some(vocab::XSD_DATETIME), None),
        ),
        (
            v("x"),
            v("l"),
            Value::literal("2026-03-01T09:00:00", Some(vocab::XSD_DATETIME), None),
        ),
    ]);
    let p = params(&[("x", node_ref(v("x")))]);
    let r = t.qp(
        "MATCH (n) WHERE n = $x RETURN n.a = n.b AS eq, n.a AS a, n.b AS b",
        &p,
    );
    assert_eq!(r.rows[0][0], b(true));
    let j = r.to_json();
    assert_eq!(j["rows"][0][1], "2026-03-01T12:00:00.000+02:00");
    assert_eq!(j["rows"][0][2], "2026-03-01T10:00:00.000Z");
    assert_eq!(
        t.q("MATCH (n {a: datetime('2026-03-01T10:00:00Z')}) RETURN n")
            .rows
            .len(),
        1
    );
    let r = t.qp("MATCH (n) WHERE n = $x RETURN n.l AS l", &p);
    assert!(matches!(r.rows[0][0], CypherValue::LocalDateTime(_)));
    assert_eq!(r.to_json()["rows"][0][0], "2026-03-01T09:00:00.000");
}

// "Node and relationship values"
#[test]
fn node_and_relationship_values() {
    let t = T::new();
    let mut e1 = None;
    t.tx(|tx| {
        tx.assert(v("alice"), rdf_type(), v("Person"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("name"), sv("Alice"), Valid::ALWAYS)?;
        e1 = Some(
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    let r = t.q("MATCH (a:Person)-[r:worksAt]->(c) RETURN a, r");
    let CypherValue::Node(n) = &r.rows[0][0] else {
        panic!()
    };
    assert_eq!(n.element_id, "urn:tiramemsu:v:alice");
    assert_eq!(n.labels, vec!["Person"]);
    assert_eq!(n.properties["name"], s("Alice"));
    let CypherValue::Relationship(rel) = &r.rows[0][1] else {
        panic!()
    };
    assert_eq!(rel.rel_type, "worksAt");
    assert_eq!(rel.start_element_id, "urn:tiramemsu:v:alice");
    assert_eq!(rel.end_element_id, "urn:tiramemsu:v:acme");
    assert_eq!(rel.eid, e1.unwrap());
    let j = r.to_json();
    assert_eq!(j["rows"][0][0]["labels"][0], "Person");
    assert_eq!(j["rows"][0][1]["type"], "worksAt");
}

// "Two-hop named path"
#[test]
fn fixed_length_named_path() {
    let t = T::new();
    t.assert(&[(v("a"), v("knows"), v("b")), (v("b"), v("knows"), v("c"))]);
    let r = t.q(
        "MATCH p = (x)-[:knows]->()-[:knows]->(z) RETURN length(p) AS l, [n IN nodes(p) | elementId(n)] AS ids",
    );
    assert_eq!(r.rows[0][0], i(2));
    assert_eq!(
        r.rows[0][1],
        list(vec![
            s("urn:tiramemsu:v:a"),
            s("urn:tiramemsu:v:b"),
            s("urn:tiramemsu:v:c")
        ])
    );
}

// volatile virtual properties
#[test]
fn volatile_properties() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), sv("Alice"))]);
    let dt = |s: &str| Value::literal(s, Some(vocab::XSD_DATETIME), None);
    t.tx(|tx| {
        tx.set_volatile(v("alice"), v("lastSeen"), dt("2026-09-29T10:00:00Z"))?;
        Ok(())
    });
    let p = params(&[("alice", node_ref(v("alice")))]);
    let r = t.qp("MATCH (n) WHERE n = $alice RETURN n.lastSeen", &p);
    assert_eq!(r.to_json()["rows"][0][0], "2026-09-29T10:00:00.000Z");
    let r = t.qp("MATCH (n) WHERE n = $alice RETURN keys(n) AS k", &p);
    assert_eq!(r.rows[0][0], list(vec![s("lastSeen"), s("name")]));
    // absent in the past
    let r =
        t.db.as_of(TimeRef::Tx(1))
            .cypher("MATCH (n) WHERE n = $alice RETURN n.lastSeen", &p)
            .unwrap();
    assert_eq!(r.rows[0][0], null());
    // the triple wins
    t.assert(&[(v("alice"), v("lastSeen"), dt("2020-01-01T00:00:00Z"))]);
    let r = t.qp("MATCH (n) WHERE n = $alice RETURN n.lastSeen", &p);
    assert_eq!(r.to_json()["rows"][0][0], "2020-01-01T00:00:00.000Z");
}

// "db.labels" / "Unknown procedure"
#[test]
fn procedures() {
    let t = T::new();
    t.assert(&[
        (v("a"), rdf_type(), v("Person")),
        (v("b"), rdf_type(), v("Company")),
        (v("a"), v("worksAt"), v("b")),
        (v("a"), v("name"), sv("A")),
    ]);
    let r = t.q("CALL db.labels() YIELD label RETURN label ORDER BY label");
    assert_eq!(r.rows, vec![vec![s("Company")], vec![s("Person")]]);
    let r = t.q("CALL db.relationshipTypes() YIELD relationshipType RETURN relationshipType");
    assert_eq!(r.rows, vec![vec![s("worksAt")]]);
    let r = t.q("CALL db.propertyKeys() YIELD propertyKey RETURN propertyKey");
    assert_eq!(r.rows, vec![vec![s("name")]]);
    assert!(
        matches!(t.qerr("CALL dbms.components()"), Error::Unsupported { ref feature } if feature.contains("dbms.components"))
    );
}

// "Type error in arithmetic" / "Integer division by zero" / "Lenient conversion"
#[test]
fn runtime_errors() {
    let t = T::new();
    assert!(matches!(t.qerr("RETURN 'a' - 1"), Error::Eval { .. }));
    assert!(matches!(t.qerr("RETURN 1 / 0"), Error::Eval { .. }));
    assert!(matches!(t.qerr("RETURN 1 % 0"), Error::Eval { .. }));
    assert_eq!(t.one("RETURN toInteger('x') AS v"), null());
    assert_eq!(t.one("RETURN 1.0 / 0 AS v"), f(f64::INFINITY));
}

// "Unclosed parenthesis" / "Span refers to original text after extensions" via the facade
#[test]
fn parse_errors_carry_spans() {
    let t = T::new();
    let e = t.qerr("MATCH (n:Person RETURN n");
    match e {
        Error::Parse { dialect, span, .. } => {
            assert_eq!(dialect, Dialect::Cypher);
            assert_eq!(span.unwrap().offset, 6);
        }
        other => panic!("{other:?}"),
    }
    match t.qerr("USE AS OF 3 MATCH (n RETURN n") {
        Error::Parse { span, .. } => assert_eq!(span.unwrap().offset, 18),
        other => panic!("{other:?}"),
    }
}

// unsupported features and path constructs through the facade
#[test]
fn unsupported_features() {
    let t = T::new();
    for (q, what) in [
        ("MATCH (n) FOREACH (x IN [1] | SET n.a = x)", "FOREACH"),
        ("LOAD CSV FROM 'file:///x.csv' AS row RETURN row", "LOAD CSV"),
        ("MATCH (n:!Person) RETURN n", "label expression"),
        ("MATCH (a {name:'Alice'})-[:knows*1..3]->(b) RETURN b", "variable-length"),
        ("MATCH p = shortestPath((a)-[:knows*]-(b)) WHERE a.name = 'A' AND b.name = 'B' RETURN p", "shortestPath"),
    ] {
        match t.qerr(q) {
            Error::Unsupported { feature } => assert!(feature.contains(what), "{q}: {feature}"),
            other => panic!("{q}: {other:?}"),
        }
    }
}
