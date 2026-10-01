//! `named-graphs` through SPARQL Update: GRAPH blocks, WITH, USING, membership
//! deletion and the graph-management operations.
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

const IN_GRAPH: &str = "urn:tiramemsu:sys:inGraph";

fn g(n: &str) -> Value {
    Value::iri(format!("urn:g:{n}"))
}

fn members(t: &T, graph: &str) -> Vec<Option<Value>> {
    t.col(
        &format!("SELECT ?s WHERE {{ GRAPH <urn:g:{graph}> {{ ?s ?p ?o }} }} ORDER BY ?s"),
        "s",
    )
}

/// `(v:a v:p v:b)` in `<g1>` and `<g2>`, `(v:c v:p v:d)` in `<g2>`, `(v:e v:p v:f)` in none.
fn store() -> T {
    let t = T::new();
    t.upd("INSERT DATA { GRAPH <urn:g:1> { v:a v:p v:b } GRAPH <urn:g:2> { v:a v:p v:b . v:c v:p v:d } }");
    t.upd("INSERT DATA { v:e v:p v:f }");
    t
}

// @lat: [[tests#Named Graphs#SPARQL Insert Into A Graph]]
#[test]
fn insert_into_a_graph() {
    // a new graph: one statement, one membership, reported separately
    let t = T::new();
    let r = t.upd("INSERT DATA { GRAPH <urn:g:1> { v:a v:p v:b } }");
    assert_eq!((r.asserted.len(), r.memberships.len()), (1, 1));
    assert_eq!(members(&t, "1"), some(&[v("a")]));
    // idempotent
    let again = t.upd("INSERT DATA { GRAPH <urn:g:1> { v:a v:p v:b } }");
    assert_eq!((again.asserted.len(), again.memberships.len()), (0, 0));
    // an already-live statement keeps its eid and gains one membership
    let t = T::new();
    let first = t.upd("INSERT DATA { v:a v:p v:b }");
    let r = t.upd("INSERT DATA { GRAPH <urn:g:1> { v:a v:p v:b } }");
    assert_eq!((r.asserted.len(), r.memberships.len()), (0, 1));
    assert_eq!(t.live().len(), 2, "one statement and one membership");
    assert!(first.asserted.len() == 1);
    // one statement in two graphs keeps one eid
    let t = store();
    let stmts =
        t.db.now()
            .triples(
                None,
                Some(t.db.now().encode(&v("p")).unwrap().unwrap()),
                None,
            )
            .unwrap();
    assert_eq!(stmts.len(), 3);
    // a graph variable bound by WHERE
    t.upd("INSERT { GRAPH ?g { v:audit v:saw ?s } } WHERE { GRAPH ?g { ?s v:p ?o } }");
    assert_eq!(
        t.col(
            "SELECT ?s WHERE { GRAPH <urn:g:1> { v:audit v:saw ?s } }",
            "s"
        ),
        some(&[v("a")])
    );
    assert_eq!(
        t.col(
            "SELECT ?s WHERE { GRAPH <urn:g:2> { v:audit v:saw ?s } } ORDER BY ?s",
            "s"
        ),
        some(&[v("a"), v("c")])
    );
    // an unbound graph variable in a template is a parse error before anything runs
    let before = t.last_t();
    let (_, msg) =
        assert_parse(t.err("INSERT { GRAPH ?nope { v:x v:p v:y } } WHERE { ?s v:p ?o }"));
    assert!(msg.contains("?nope"), "{msg}");
    assert_eq!(t.last_t(), before);
}

// @lat: [[tests#Named Graphs#SPARQL Delete From A Graph]]
#[test]
fn delete_from_a_graph() {
    let t = store();
    // removing from one graph keeps the statement and its other membership
    let r = t.upd("DELETE DATA { GRAPH <urn:g:1> { v:a v:p v:b } }");
    assert_eq!((r.memberships_retracted.len(), r.retracted.len()), (1, 0));
    assert!(t.has(&v("a"), &v("p"), &v("b")));
    assert!(members(&t, "1").is_empty());
    assert_eq!(members(&t, "2"), some(&[v("a"), v("c")]));
    // removing from a graph the statement is not in: a no-op
    let r = t.upd("DELETE DATA { GRAPH <urn:g:1> { v:c v:p v:d } }");
    assert!(r.retracted.is_empty() && r.memberships_retracted.is_empty());
    let r = t.upd("DELETE DATA { GRAPH <urn:never:seen> { v:c v:p v:d } }");
    assert!(r.retracted.is_empty() && r.memberships_retracted.is_empty());
    // a template delete in a graph removes only memberships
    t.upd("DELETE { GRAPH <urn:g:2> { ?s ?p ?o } } WHERE { GRAPH <urn:g:2> { ?s ?p ?o } }");
    assert!(members(&t, "2").is_empty());
    assert_eq!(t.sel("SELECT ?s WHERE { ?s v:p ?o }").rows.len(), 3);
    // a plain delete removes the statement and every membership
    let t = store();
    let r = t.upd("DELETE DATA { v:a v:p v:b }");
    assert_eq!((r.retracted.len(), r.memberships_retracted.len()), (1, 2));
    assert!(members(&t, "1").is_empty());
    assert_eq!(members(&t, "2"), some(&[v("c")]));
}

// @lat: [[tests#Named Graphs#SPARQL WITH And USING]]
#[test]
fn with_and_using() {
    // WITH sets the graph of a template
    let t = T::new();
    t.upd("WITH <urn:g:1> INSERT { v:a v:p v:b } WHERE {}");
    assert_eq!(members(&t, "1"), some(&[v("a")]));
    // WITH DELETE removes memberships of the WITH graph, and only those
    let t = store();
    t.upd("WITH <urn:g:1> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }");
    assert!(members(&t, "1").is_empty());
    assert!(t.has(&v("a"), &v("p"), &v("b")) && t.has(&v("e"), &v("p"), &v("f")));
    assert_eq!(members(&t, "2"), some(&[v("a"), v("c")]));
    // USING scopes the WHERE default graph
    let t = store();
    t.upd("INSERT { ?s v:seen true } USING <urn:g:2> WHERE { ?s v:p ?o }");
    assert_eq!(
        t.col("SELECT ?s WHERE { ?s v:seen true } ORDER BY ?s", "s"),
        some(&[v("a"), v("c")])
    );
    // USING NAMED restricts GRAPH in the WHERE
    let t = store();
    t.upd("INSERT { ?s v:only1 true } USING NAMED <urn:g:1> WHERE { GRAPH ?g { ?s v:p ?o } }");
    assert_eq!(
        t.col("SELECT ?s WHERE { ?s v:only1 true }", "s"),
        some(&[v("a")])
    );
    // a time IRI in GRAPH of a data block is still a Parse error naming SERVICE
    let (_, msg) =
        assert_parse(t.err("INSERT DATA { GRAPH <urn:tiramemsu:tm:asOf/150> { v:a v:b v:c } }"));
    assert!(msg.contains("SERVICE"), "{msg}");
}

// @lat: [[tests#Named Graphs#SPARQL Graph Management]]
#[test]
fn graph_management() {
    // CLEAR keeps the statements
    let t = store();
    t.upd("CLEAR GRAPH <urn:g:1>");
    assert!(t.has(&v("a"), &v("p"), &v("b")));
    assert!(members(&t, "1").is_empty());
    assert_eq!(members(&t, "2"), some(&[v("a"), v("c")]));
    // CREATE makes an empty graph visible as a declaration only
    t.upd("CREATE GRAPH <urn:g:9>");
    let decl = "SELECT ?g WHERE { ?g a <urn:tiramemsu:sys:Graph> }";
    assert_eq!(t.col(decl, "g"), some(&[g("9")]));
    assert!(!t
        .col("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }", "g")
        .contains(&Some(g("9"))));
    assert!(matches!(
        t.err("CREATE GRAPH <urn:g:9>"),
        Error::GraphExists { .. }
    ));
    t.upd("CREATE SILENT GRAPH <urn:g:9>");
    // DROP retracts memberships and the declaration, and keeps other metadata
    t.upd("INSERT DATA { <urn:g:2> v:startedBy v:agent7 . <urn:g:9> v:startedBy v:agent8 }");
    t.upd("CREATE GRAPH <urn:g:2>");
    t.upd("DROP GRAPH <urn:g:2>");
    t.upd("DROP GRAPH <urn:g:9>");
    assert!(members(&t, "2").is_empty());
    assert!(t.col(decl, "g").is_empty());
    assert!(t.has(&g("2"), &v("startedBy"), &v("agent7")));
    assert!(t.has(&v("a"), &v("p"), &v("b")) && t.has(&v("c"), &v("p"), &v("d")));
    // a missing graph is GraphNotFound, and SILENT succeeds
    let before = t.last_t();
    match t.err("CLEAR GRAPH <urn:never:used>") {
        Error::GraphNotFound { graph } => assert!(graph.contains("urn:never:used")),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        t.err("DROP GRAPH <urn:never:used>"),
        Error::GraphNotFound { .. }
    ));
    let r = t.upd("CLEAR SILENT GRAPH <urn:never:used>");
    assert!(r.retracted.is_empty() && r.memberships_retracted.is_empty());
    assert_eq!(t.last_t(), before + 1, "only the SILENT request committed");
    // CLEAR NAMED and DROP NAMED reach every graph
    let t = store();
    t.upd("CLEAR NAMED");
    assert!(t
        .sel("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }")
        .rows
        .is_empty());
    assert_eq!(t.sel("SELECT ?s WHERE { ?s v:p ?o }").rows.len(), 3);
    let t = store();
    t.upd("CREATE GRAPH <urn:g:9>");
    t.upd("DROP NAMED");
    assert!(t
        .col("SELECT ?g WHERE { ?g a <urn:tiramemsu:sys:Graph> }", "g")
        .is_empty());
    assert_eq!(t.sel("SELECT ?s WHERE { ?s v:p ?o }").rows.len(), 3);
}

// @lat: [[tests#Named Graphs#Graph Updates Fail Atomically]]
#[test]
fn graph_updates_fail_atomically() {
    let t = store();
    let before = t.last_t();
    let e = t.err(
        "INSERT DATA { v:x v:p v:y } ; INSERT DATA { GRAPH <urn:tiramemsu:tx:1> { v:c v:p v:d } }",
    );
    assert!(matches!(e, Error::InvalidGraphName { .. }), "{e:?}");
    assert!(!t.has(&v("x"), &v("p"), &v("y")));
    assert_eq!(t.last_t(), before);
    // a rejected operation aborts the whole request, before anything is written
    for (q, want) in [
        (
            "INSERT DATA { v:x v:p v:y } ; COPY <urn:g:1> TO <urn:g:3>",
            "COPY",
        ),
        (
            "INSERT DATA { v:x v:p v:y } ; MOVE <urn:g:1> TO <urn:g:3>",
            "MOVE",
        ),
        (
            "INSERT DATA { v:x v:p v:y } ; ADD <urn:g:1> TO <urn:g:3>",
            "ADD",
        ),
        ("CREATE GRAPH <urn:g:7> ; DROP DEFAULT", "DROP DEFAULT"),
    ] {
        assert_unsupported(t.err(q), want);
    }
    assert!(!t.has(&v("x"), &v("p"), &v("y")));
    assert_eq!(t.last_t(), before);
}

// @lat: [[tests#Named Graphs#SPARQL Membership Predicate Is Engine Owned]]
#[test]
fn membership_predicate_is_engine_owned() {
    let t = store();
    let before = t.last_t();
    let e = t.err(&format!(
        "INSERT DATA {{ <urn:tiramemsu:stmt:1> <{IN_GRAPH}> <urn:g:9> }}"
    ));
    assert!(
        matches!(&e, Error::ReservedNamespace(m) if m == IN_GRAPH),
        "{e:?}"
    );
    // schema statements cannot be members
    let e = t.err("INSERT DATA { GRAPH <urn:g:1> { v:email <urn:tiramemsu:sys:unique> true } }");
    assert!(matches!(e, Error::ReservedNamespace(_)), "{e:?}");
    assert!(!t.has(
        &v("email"),
        &Value::iri(vocab::SYS_UNIQUE),
        &Value::Bool(true)
    ));
    assert_eq!(t.last_t(), before);
    // memberships are readable
    let gs = t.col(
        &format!("SELECT DISTINCT ?g WHERE {{ ?e <{IN_GRAPH}> ?g }} ORDER BY ?g"),
        "g",
    );
    assert_eq!(gs, some(&[g("1"), g("2")]));
}

// @lat: [[tests#Named Graphs#Cardinality One Drops Old Memberships]]
#[test]
fn cardinality_one_drops_old_memberships() {
    let t = T::new();
    t.upd("INSERT DATA { v:age <urn:tiramemsu:sys:cardinality> <urn:tiramemsu:sys:one> }");
    t.upd("INSERT DATA { GRAPH <urn:g:1> { v:alice v:age 41 } }");
    let r = t.upd("INSERT DATA { GRAPH <urn:g:1> { v:alice v:age 42 } }");
    assert_eq!(r.retracted, vec![(r.retracted[0].0, RetKind::Cardinality)]);
    assert_eq!(r.memberships_retracted.len(), 1);
    assert_eq!(r.memberships_retracted[0].1, RetKind::Cardinality);
    assert_eq!(
        t.col(
            "SELECT ?a WHERE { GRAPH <urn:g:1> { v:alice v:age ?a } }",
            "a"
        ),
        some(&[int(42)])
    );
}
