//! Vector writes and index commands (OpenSpec change `add-vector-query-writes`):
//! dropping a model, one definition per IRI, view-aware definitions, and the
//! SPARQL Update and Cypher write surfaces.

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn open() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("m.db"), OpenOptions::default()).unwrap();
    (dir, db)
}

fn tx(db: &Db, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> TxId {
    let mut f = Some(f);
    db.transact(TxOptions::default(), |tx| (f.take().unwrap())(tx))
        .unwrap()
        .t
}

fn search(view: &View<'_>, model: &Value, q: &[f32]) -> Result<Vec<Eid>> {
    let Some(m) = view.encode(model)? else {
        return Err(Error::invalid_query("unknown model"));
    };
    Ok(view
        .vector_search(&VectorQuery::new(m, q.to_vec()))?
        .hits
        .iter()
        .map(|h| h.eid)
        .collect())
}

fn embedded_fact(db: &Db, model: &Value, o: &str, vec: &[f32]) -> (Eid, TxId) {
    let mut eid = None;
    let t = tx(db, |tx| {
        let e = tx
            .assert(v("alice"), v("likes"), v(o), Valid::ALWAYS)?
            .eid();
        tx.set_embedding(e, model, vec)?;
        eid = Some(e);
        Ok(())
    });
    (eid.unwrap(), t)
}

// @tg: verifies:: [[tests#Vector Recall#Dropping A Model Keeps Its Past]] {since: "2026-10-12", level: unit}
#[test]
fn dropping_a_model_keeps_its_past() {
    let (_d, db) = open();
    let m = v("m");
    tx(&db, |tx| {
        tx.define_embedding_model(&m, &EmbeddingModel::new(2))
            .map(|_| ())
    });
    let (tea, before) = embedded_fact(&db, &m, "tea", &[1.0, 0.0]);
    let mut dropped = None;
    tx(&db, |tx| {
        dropped = Some(tx.drop_embedding_model(&m)?);
        Ok(())
    });
    assert_eq!(dropped, Some(true));
    // now: undefined; before the drop: the same answer as then
    assert!(matches!(
        search(&db.now(), &m, &[1.0, 0.0]),
        Err(Error::InvalidQuery { .. })
    ));
    assert_eq!(
        search(&db.as_of(TimeRef::Tx(before.0)), &m, &[1.0, 0.0]).unwrap(),
        vec![tea]
    );
    let r = db.transact(TxOptions::default(), |tx| {
        tx.set_embedding(tea, &m, &[1.0, 0.0]).map(|_| ())
    });
    assert!(matches!(r, Err(Error::InvalidQuery { .. })));
    // a second drop writes nothing
    let mut again = None;
    let report = db
        .transact(TxOptions::default(), |tx| {
            again = Some(tx.drop_embedding_model(&m)?);
            Ok(())
        })
        .unwrap();
    assert_eq!(again, Some(false));
    assert!(report.retracted.is_empty());
    // a different definition is refused, the identical one restores the vectors
    let r = db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(&m, &EmbeddingModel::new(3))
            .map(|_| ())
    });
    assert!(matches!(r, Err(Error::InvalidQuery { .. })));
    tx(&db, |tx| {
        tx.define_embedding_model(&m, &EmbeddingModel::new(2))
            .map(|_| ())
    });
    assert_eq!(search(&db.now(), &m, &[1.0, 0.0]).unwrap(), vec![tea]);
}

// @tg: verifies:: [[tests#Vector Recall#Definitions Are Read In The View]] {since: "2026-10-12", level: unit}
#[test]
fn definitions_are_read_in_the_view() {
    let (_d, db) = open();
    let m = v("m");
    let t1 = tx(&db, |tx| {
        tx.assert(v("alice"), v("likes"), v("tea"), Valid::ALWAYS)
            .map(|_| ())
    });
    tx(&db, |tx| {
        tx.define_embedding_model(&m, &EmbeddingModel::new(2))
            .map(|_| ())
    });
    assert!(search(&db.now(), &m, &[1.0, 0.0]).unwrap().is_empty());
    assert!(matches!(
        search(&db.as_of(TimeRef::Tx(t1.0)), &m, &[1.0, 0.0]),
        Err(Error::InvalidQuery { .. })
    ));
}

#[cfg(feature = "sparql")]
fn sparql_ok(db: &Db, q: &str) {
    db.now().sparql(q).unwrap_or_else(|e| panic!("{q}: {e}"));
}

// @tg: verifies:: [[tests#Vector Recall#SPARQL Update Defines Embeds And Drops]] {since: "2026-10-12", level: unit}
#[cfg(feature = "sparql")]
#[test]
fn sparql_update_defines_embeds_and_drops() {
    let (_d, db) = open();
    let m = Value::iri("urn:ex:m");
    // define, assert and embed in one request
    sparql_ok(
        &db,
        "PREFIX sys: <urn:tiramemsu:sys:> \
         INSERT DATA { <urn:ex:m> sys:dimensions 3 ; sys:metric \"dot\" . \
           v:alice v:likes v:tea {| tm:vectorEmbedding \"[1, 0, 0]\" ; tm:vectorModel <urn:ex:m> |} }",
    );
    let def = db
        .transact(TxOptions::default(), |tx| {
            assert_eq!(
                tx.embedding_model(&m)?,
                Some(EmbeddingModel {
                    dimensions: 3,
                    metric: Metric::Dot,
                    encoding: VectorEncoding::F32
                })
            );
            Ok(())
        })
        .unwrap();
    assert!(def.asserted.is_empty());
    let tea = search(&db.now(), &m, &[1.0, 0.0, 0.0]).unwrap();
    assert_eq!(tea.len(), 1);
    // embed matched statements
    sparql_ok(
        &db,
        "INSERT DATA { v:alice v:likes v:coffee . v:bob v:likes v:coffee }",
    );
    sparql_ok(
        &db,
        "INSERT { ?e tm:vectorEmbedding \"[0, 1, 0]\" ; tm:vectorModel <urn:ex:m> } \
         WHERE { ?s v:likes v:coffee ~ ?e }",
    );
    let hits = search(&db.now(), &m, &[0.0, 1.0, 0.0]).unwrap();
    assert_eq!(hits.len(), 3);
    assert!(!hits[..2].contains(&tea[0]));
    // the SPARQL search sees the same
    let res = db
        .now()
        .sparql("SELECT ?e WHERE { ?e tm:vectorMatch \"[1, 0, 0]\" ; tm:vectorModel <urn:ex:m> ; tm:vectorLimit 1 }")
        .unwrap();
    assert_eq!(
        res.solutions().unwrap().rows,
        vec![vec![Some(Value::Stmt(tea[0]))]]
    );
    // errors: embeddings are append-only, no graphs, a subject must be a statement
    for (bad, unsupported) in [
        ("DELETE DATA { <urn:tiramemsu:stmt:1> tm:vectorModel <urn:ex:m> }", true),
        ("DELETE { ?e tm:vectorEmbedding ?x } WHERE { ?s v:likes ?o ~ ?e }", true),
        ("INSERT DATA { GRAPH v:g { v:alice v:likes v:tea {| tm:vectorEmbedding \"[1,0,0]\" ; tm:vectorModel <urn:ex:m> |} } }", false),
        ("INSERT DATA { v:alice tm:vectorEmbedding \"[1,0,0]\" ; tm:vectorModel <urn:ex:m> }", false),
        ("INSERT DATA { v:a v:p v:b {| tm:vectorEmbedding \"[1,0]\" ; tm:vectorModel <urn:ex:m> |} }", false),
        ("INSERT DATA { v:a v:p v:b {| tm:vectorEmbedding \"[1,0,0]\" |} }", false),
        ("PREFIX sys: <urn:tiramemsu:sys:> INSERT DATA { <urn:ex:n> sys:metric \"dot\" }", false),
    ] {
        let r = db.now().sparql(bad);
        if unsupported {
            assert!(matches!(r, Err(Error::Unsupported { .. })), "{bad}: {r:?}");
        } else {
            assert!(matches!(r, Err(Error::InvalidQuery { .. })), "{bad}: {r:?}");
        }
    }
    assert!(db.now().encode(&v("a")).unwrap().is_none());
    // a non-matching definition triple is a no-op; a matching one drops the model
    sparql_ok(
        &db,
        "PREFIX sys: <urn:tiramemsu:sys:> DELETE DATA { <urn:ex:m> sys:dimensions 4 }",
    );
    assert_eq!(search(&db.now(), &m, &[1.0, 0.0, 0.0]).unwrap().len(), 3);
    sparql_ok(
        &db,
        "PREFIX sys: <urn:tiramemsu:sys:> DELETE DATA { <urn:ex:m> sys:dimensions 3 }",
    );
    assert!(matches!(
        search(&db.now(), &m, &[1.0, 0.0, 0.0]),
        Err(Error::InvalidQuery { .. })
    ));
    let defs = db
        .now()
        .sparql("PREFIX sys: <urn:tiramemsu:sys:> SELECT ?p WHERE { <urn:ex:m> ?p ?o }")
        .unwrap();
    assert!(defs.solutions().unwrap().rows.is_empty());
}

#[cfg(feature = "cypher")]
fn cw(db: &Db, q: &str) -> Result<CypherResult> {
    db.cypher_write(TxOptions::default(), q, &CypherParams::default())
}

// @tg: verifies:: [[tests#Vector Recall#Cypher Creates Embeds And Drops Indexes]] {since: "2026-10-12", level: unit}
#[cfg(feature = "cypher")]
#[test]
fn cypher_creates_embeds_and_drops_indexes() {
    let (_d, db) = open();
    let none = CypherParams::default();
    let m = v("m");
    // a read handle refuses DDL and setEmbedding before anything runs
    for q in [
        "CREATE VECTOR INDEX m OPTIONS {dimensions: 3}",
        "DROP INDEX m",
        "MATCH ()-[r]->() CALL tiramemsu.vector.setEmbedding(r, 'm', [1, 0, 0]) YIELD stored RETURN stored",
    ] {
        assert!(
            matches!(db.now().cypher(q, &none), Err(Error::Unsupported { .. })),
            "{q}"
        );
    }
    let created = cw(&db, "CREATE VECTOR INDEX m OPTIONS {dimensions: 3}").unwrap();
    assert!(created.rows.is_empty());
    assert!(matches!(
        cw(&db, "CREATE VECTOR INDEX m OPTIONS {dimensions: 3}"),
        Err(Error::InvalidQuery { .. })
    ));
    cw(
        &db,
        "CREATE VECTOR INDEX m IF NOT EXISTS OPTIONS {dimensions: 3}",
    )
    .unwrap();
    assert!(matches!(
        cw(
            &db,
            "CREATE VECTOR INDEX m IF NOT EXISTS OPTIONS {dimensions: 4}"
        ),
        Err(Error::InvalidQuery { .. })
    ));
    cw(
        &db,
        "CREATE (:P {name: 'a'})-[:likes]->(:P {name: 'b'}), (:P {name: 'c'})-[:likes]->(:P {name: 'd'})",
    )
    .unwrap();
    let stored = cw(
        &db,
        "MATCH (a)-[r:likes]->(b) WHERE a.name = 'a' \
         CALL tiramemsu.vector.setEmbedding(r, 'm', [1, 0, 0]) YIELD stored RETURN stored",
    )
    .unwrap();
    assert_eq!(stored.rows, vec![vec![CypherValue::Boolean(true)]]);
    cw(
        &db,
        "MATCH (a)-[r:likes]->(b) WHERE a.name = 'c' \
         CALL tiramemsu.vector.setEmbedding(r, 'm', [0, 1, 0]) YIELD stored RETURN stored",
    )
    .unwrap();
    let again = cw(
        &db,
        "MATCH (a)-[r:likes]->(b) WHERE a.name = 'a' \
         CALL tiramemsu.vector.setEmbedding(r, 'm', [1, 0, 0]) YIELD stored RETURN stored",
    )
    .unwrap();
    assert_eq!(again.rows, vec![vec![CypherValue::Boolean(false)]]);
    let rust = search(&db.now(), &m, &[1.0, 0.1, 0.0]).unwrap();
    assert_eq!(rust.len(), 2);
    let hits = db
        .now()
        .cypher(
            "CALL tiramemsu.vector.search('m', [1, 0.1, 0]) YIELD statement RETURN statement",
            &none,
        )
        .unwrap();
    let ids: Vec<String> = hits
        .rows
        .iter()
        .map(|r| match &r[0] {
            CypherValue::Node(n) => n.element_id.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    let want: Vec<String> = rust
        .iter()
        .map(|e| format!("urn:tiramemsu:stmt:{}", e.n()))
        .collect();
    assert_eq!(ids, want);
    let shown = db.now().cypher("SHOW VECTOR INDEXES", &none).unwrap();
    assert_eq!(
        shown.columns,
        ["name", "model", "dimensions", "metric", "encoding"]
    );
    assert_eq!(
        shown.rows,
        vec![vec![
            CypherValue::String("m".into()),
            CypherValue::String("urn:tiramemsu:v:m".into()),
            CypherValue::Integer(3),
            CypherValue::String("cosine".into()),
            CypherValue::String("f32".into()),
        ]]
    );
    // drop; drop again needs IF EXISTS; search fails now
    cw(&db, "DROP INDEX m").unwrap();
    assert!(matches!(
        cw(&db, "DROP VECTOR INDEX m"),
        Err(Error::InvalidQuery { .. })
    ));
    cw(&db, "DROP VECTOR INDEX m IF EXISTS").unwrap();
    assert!(db
        .now()
        .cypher("CALL tiramemsu.vector.search('m', [1, 0, 0])", &none)
        .is_err());
    assert!(db
        .now()
        .cypher("SHOW INDEXES", &none)
        .unwrap()
        .rows
        .is_empty());
    // Neo4j options and a parameter map
    cw(
        &db,
        "CREATE VECTOR INDEX `urn:ex:neo` OPTIONS {indexConfig: \
         {`vector.dimensions`: 8, `vector.similarity_function`: 'euclidean'}}",
    )
    .unwrap();
    let mut params = CypherParams::default();
    params.insert(
        "opts".into(),
        CypherValue::Map(
            [
                ("dimensions".to_string(), CypherValue::Integer(2)),
                ("encoding".to_string(), CypherValue::String("f16".into())),
            ]
            .into_iter()
            .collect(),
        ),
    );
    db.cypher_write(
        TxOptions::default(),
        "CREATE VECTOR INDEX p OPTIONS $opts",
        &params,
    )
    .unwrap();
    db.transact(TxOptions::default(), |tx| {
        assert_eq!(
            tx.embedding_model(Value::iri("urn:ex:neo"))?,
            Some(EmbeddingModel {
                dimensions: 8,
                metric: Metric::L2,
                encoding: VectorEncoding::F32
            })
        );
        assert_eq!(
            tx.embedding_model(v("p"))?.map(|d| d.encoding),
            Some(VectorEncoding::F16)
        );
        Ok(())
    })
    .unwrap();
    for bad in [
        "CREATE VECTOR INDEX x OPTIONS {}",
        "CREATE VECTOR INDEX x OPTIONS {dimensions: 2, metric: 'manhattan'}",
        "CREATE VECTOR INDEX x OPTIONS {dimensions: 2, size: 3}",
    ] {
        assert!(cw(&db, bad).is_err(), "{bad}");
    }
    assert!(matches!(
        cw(
            &db,
            "CREATE VECTOR INDEX x FOR ()-[r]-() ON (r.e) OPTIONS {dimensions: 2}"
        ),
        Err(Error::Unsupported { .. })
    ));
}

// @tg: verifies:: [[tests#Vector Recall#SPARQL And Cypher Writes Agree]] {since: "2026-10-12", level: unit}
#[cfg(all(feature = "sparql", feature = "cypher"))]
#[test]
fn sparql_and_cypher_writes_agree() {
    let (_d, db) = open();
    cw(
        &db,
        "CREATE (:P {name: 'x'})-[:likes]->(:P {name: 'tea'}),                 (:P {name: 'y'})-[:likes]->(:P {name: 'coffee'}),                 (:P {name: 'z'})-[:likes]->(:P {name: 'milk'})",
    )
    .unwrap();
    // one model defined and filled through SPARQL, one through Cypher
    sparql_ok(
        &db,
        "PREFIX sys: <urn:tiramemsu:sys:> INSERT DATA { v:s sys:dimensions 2 }",
    );
    cw(&db, "CREATE VECTOR INDEX c OPTIONS {dimensions: 2}").unwrap();
    for (o, vec) in [
        ("tea", "[1, 0]"),
        ("coffee", "[0.6, 0.8]"),
        ("milk", "[0, 1]"),
    ] {
        sparql_ok(
            &db,
            &format!(
                "INSERT {{ ?e tm:vectorEmbedding \"{vec}\" ; tm:vectorModel v:s }} \
                 WHERE {{ ?a v:likes ?b ~ ?e . ?b v:name \"{o}\" }}"
            ),
        );
        let r = cw(
            &db,
            &format!(
                "MATCH ()-[r:likes]->(b {{name: '{o}'}}) \
                 CALL tiramemsu.vector.setEmbedding(r, 'c', {vec}) YIELD stored RETURN stored"
            ),
        )
        .unwrap();
        assert_eq!(r.rows, vec![vec![CypherValue::Boolean(true)]]);
    }
    let by_sparql = search(&db.now(), &v("s"), &[0.9, 0.2]).unwrap();
    let by_cypher = search(&db.now(), &v("c"), &[0.9, 0.2]).unwrap();
    assert_eq!(by_sparql.len(), 3);
    assert_eq!(by_sparql, by_cypher);
}
