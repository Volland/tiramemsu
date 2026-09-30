//! Semantic analysis tests: scopes, kinds, aggregates, parameters, shadowing,
//! and the write-related checks.

use tm_cypher::{compile, CompileCtx, CypherError, CypherParams, CypherValue, Vocab};

fn ctx(writable: bool) -> CompileCtx {
    CompileCtx {
        vocab: Vocab::default(),
        view: tm_ir::View::NOW,
        writable,
    }
}

fn err(q: &str) -> CypherError {
    compile(q, &CypherParams::new(), &ctx(false)).expect_err(q)
}

fn parse_err_text(q: &str) -> String {
    match err(q) {
        CypherError::Parse { span, .. } => q[span.start..span.end].to_string(),
        other => panic!("expected Parse for {q}, got {other:?}"),
    }
}

// cypher-read "Variable dropped by WITH is out of scope"
#[test]
fn variable_dropped_by_with() {
    let q = "MATCH (p)-[:worksAt]->(c) WITH c RETURN p";
    assert_eq!(parse_err_text(q), "p");
    let e = err(q);
    if let CypherError::Parse { span, .. } = e {
        assert_eq!(span.start, q.rfind('p').unwrap());
    }
}

// cypher-read "Undefined variable"
#[test]
fn undefined_variable() {
    assert_eq!(parse_err_text("MATCH (n) RETURN m"), "m");
}

// cypher-read "Node variable used as a relationship" / dual-view "Node variable in relationship position rejected"
#[test]
fn node_variable_in_relationship_position() {
    let q = "MATCH (n) MATCH ()-[n]->() RETURN n";
    match err(q) {
        CypherError::Parse { span, .. } => assert_eq!(span.start, q.find("[n]").unwrap() + 1),
        e => panic!("{e:?}"),
    }
    assert_eq!(
        parse_err_text("MATCH (:Belief)-[:SUPPORTED_BY]->(x) MATCH ()-[x]->() RETURN x"),
        "x"
    );
}

// cypher-dual-view: a relationship variable may stand in node position
#[test]
fn relationship_variable_in_node_position_is_accepted() {
    let q =
        "MATCH (a)-[r:WORKS_AT]->(c), (b:Belief)-[:SUPPORTED_BY]->(r) RETURN a, c, r.confidence, b";
    compile(q, &CypherParams::new(), &ctx(false)).unwrap();
    compile(
        "MATCH (a)-[r:WORKS_AT]->(c) WITH r MATCH (b)-[:SUPPORTED_BY]->(r) RETURN b",
        &CypherParams::new(),
        &ctx(false),
    )
    .unwrap();
}

// cypher-read "Aggregate in WHERE is rejected"
#[test]
fn aggregate_placement() {
    assert_eq!(
        parse_err_text("MATCH (n) WHERE count(n) > 1 RETURN n"),
        "count(n)"
    );
    assert!(matches!(
        err("RETURN count(count(*)) AS c"),
        CypherError::Parse { .. }
    ));
    compile(
        "MATCH (n) RETURN count(*) AS c, n.x AS x",
        &CypherParams::new(),
        &ctx(false),
    )
    .unwrap();
}

// cypher-read "Column mismatch"
#[test]
fn union_columns() {
    assert!(matches!(
        err("RETURN 1 AS a UNION RETURN 2 AS b"),
        CypherError::Parse { .. }
    ));
    compile(
        "RETURN 1 AS a UNION RETURN 2 AS a",
        &CypherParams::new(),
        &ctx(false),
    )
    .unwrap();
}

// cypher-read "Negative LIMIT fails"
#[test]
fn skip_limit_literals() {
    assert_eq!(parse_err_text("MATCH (n) RETURN n LIMIT -1"), "-1");
    assert!(matches!(
        err("MATCH (n) RETURN n SKIP 1.5"),
        CypherError::Parse { .. }
    ));
    let mut p = CypherParams::new();
    p.insert("s".into(), CypherValue::Integer(1));
    compile("MATCH (n) RETURN n SKIP $s LIMIT 3", &p, &ctx(false)).unwrap();
}

// cypher-read "Missing parameter"
#[test]
fn missing_parameter() {
    let e = compile(
        "MATCH (n {name: $who}) RETURN n",
        &CypherParams::new(),
        &ctx(false),
    )
    .unwrap_err();
    match e {
        CypherError::Parse { msg, span } => {
            assert!(msg.contains("who"));
            assert_eq!(
                &"MATCH (n {name: $who}) RETURN n"[span.start..span.end],
                "$who"
            );
        }
        e => panic!("{e:?}"),
    }
}

// cypher-read "Shadowing an outer variable is rejected"
#[test]
fn call_shadowing() {
    let q = "MATCH (p:Person) CALL { MATCH (p:Company) RETURN p } RETURN p";
    match err(q) {
        CypherError::Parse { span, .. } => {
            assert_eq!(&q[span.start..span.end], "p");
            assert_eq!(span.start, q.find("RETURN p }").unwrap() + 7);
        }
        e => panic!("{e:?}"),
    }
    compile(
        "MATCH (c:Company) CALL { WITH c OPTIONAL MATCH (p)-[:worksAt]->(c) RETURN count(p) AS staff } RETURN c.name, staff",
        &CypherParams::new(),
        &ctx(false),
    )
    .unwrap();
}

// cypher-read "Unknown function" / "Unknown procedure"
#[test]
fn unknown_function_and_procedure() {
    assert!(
        matches!(err("RETURN apoc.text.clean('x')"), CypherError::Unsupported { feature, .. } if feature.contains("apoc.text.clean"))
    );
    assert!(
        matches!(err("CALL dbms.components()"), CypherError::Unsupported { feature, .. } if feature.contains("dbms.components"))
    );
}

// cypher-read "Write clause on a read-only view is rejected"
#[test]
fn write_on_read_only_view() {
    match err("CREATE (n:Person {name: 'Bob'})") {
        CypherError::Unsupported { feature, .. } => assert!(feature.contains("CREATE")),
        e => panic!("{e:?}"),
    }
    assert!(matches!(
        err("MATCH (n) SET n.x = 1"),
        CypherError::Unsupported { .. }
    ));
    assert!(matches!(
        err("MATCH (n) DETACH DELETE n"),
        CypherError::Unsupported { .. }
    ));
}

// cypher-write "Query-level AS OF with a write rejected"
#[test]
fn write_under_time_clause() {
    let e = compile(
        "USE AS OF 5 MATCH (n) SET n.x = 1",
        &CypherParams::new(),
        &ctx(true),
    )
    .unwrap_err();
    assert!(matches!(e, CypherError::Unsupported { .. }));
    // a historical CALL scope feeding a SET is allowed
    compile(
        "CALL { USE AS OF 5 MATCH (a) RETURN a.title AS old } MATCH (n) SET n.title = old",
        &CypherParams::new(),
        &ctx(true),
    )
    .unwrap();
}

// cypher-write "Unsupported write features": write inside EXISTS
#[test]
fn write_inside_exists_is_unsupported() {
    let e = compile(
        "MATCH (n) WHERE EXISTS { MATCH (n)-[:X]->(m) SET m.a = 1 } RETURN n",
        &CypherParams::new(),
        &ctx(true),
    )
    .unwrap_err();
    assert!(matches!(e, CypherError::Unsupported { .. }));
}
