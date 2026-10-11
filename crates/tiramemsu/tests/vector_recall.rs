//! Vector and hybrid recall (OpenSpec changes `add-vector-recall` and
//! `add-hybrid-recall`): model definitions, append-only embeddings, exact
//! view-aware search, the prefilter, never-forget, migration and fusion.

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn model() -> Value {
    v("toy-embedder")
}

fn open() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("m.db"), OpenOptions::default()).unwrap();
    (dir, db)
}

fn define(db: &Db, def: EmbeddingModel) {
    db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(model(), &def).map(|_| ())
    })
    .unwrap();
}

/// Asserts `(alice p o)` and embeds it; returns the eid.
fn fact(db: &Db, p: &str, o: &str, vec: &[f32]) -> Eid {
    let mut eid = None;
    db.transact(TxOptions::default(), |tx| {
        let e = tx.assert(v("alice"), v(p), v(o), Valid::ALWAYS)?.eid();
        tx.set_embedding(e, model(), vec)?;
        eid = Some(e);
        Ok(())
    })
    .unwrap();
    eid.unwrap()
}

fn m(db: &Db) -> ObjectId {
    db.now().encode(&model()).unwrap().unwrap()
}

fn objects(db: &Db, view: &View<'_>, q: &VectorQuery) -> Vec<Value> {
    view.vector_search(q)
        .unwrap()
        .hits
        .iter()
        .map(|h| db.now().decode(h.o).unwrap())
        .collect()
}

// @lat: [[tests#Vector Recall#Models Are Defined Once]]
#[test]
fn models_are_defined_once() {
    let (_d, db) = open();
    let def = EmbeddingModel {
        dimensions: 4,
        metric: Metric::Dot,
        encoding: VectorEncoding::F16,
    };
    let mut first = false;
    let mut again = true;
    db.transact(TxOptions::default(), |tx| {
        first = tx.define_embedding_model(model(), &def)?;
        again = tx.define_embedding_model(model(), &def)?;
        assert_eq!(tx.embedding_model(model())?, Some(def));
        Ok(())
    })
    .unwrap();
    assert!(first && !again);
    let other = EmbeddingModel {
        dimensions: 8,
        ..def
    };
    let r = db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(model(), &other).map(|_| ())
    });
    assert!(matches!(r, Err(Error::InvalidQuery { .. })), "{r:?}");
    let r = db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(Value::str("not an iri"), &def)
            .map(|_| ())
    });
    assert!(matches!(r, Err(Error::InvalidTerm { .. })), "{r:?}");
    let r = db.transact(TxOptions::default(), |tx| {
        tx.define_embedding_model(v("zero"), &EmbeddingModel::new(0))
            .map(|_| ())
    });
    assert!(matches!(r, Err(Error::InvalidQuery { .. })), "{r:?}");
    // the definition is three engine-owned sys: statements; users cannot write them
    let r = db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("m2"),
            Value::iri(vocab::SYS_DIMENSIONS),
            Value::Int(3),
            Valid::ALWAYS,
        )
        .map(|_| ())
    });
    assert!(matches!(r, Err(Error::ReservedNamespace(_))), "{r:?}");
}

// @lat: [[tests#Vector Recall#Embeddings Are Checked And Append Only]]
#[test]
fn embeddings_are_checked_and_append_only() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(3));
    let e = fact(&db, "livesIn", "paris", &[1.0, 0.0, 0.0]);
    let set = |eid: Eid, vec: Vec<f32>| {
        let mut out = None;
        let r = db.transact(TxOptions::default(), |tx| {
            out = Some(tx.set_embedding(eid, model(), &vec)?);
            Ok(())
        });
        r.map(|_| out.unwrap())
    };
    // the same vector, scaled: identical after cosine normalisation
    assert!(!set(e, vec![2.0, 0.0, 0.0]).unwrap());
    assert!(matches!(
        set(e, vec![0.0, 1.0, 0.0]),
        Err(Error::EmbeddingConflict { .. })
    ));
    assert!(matches!(
        set(e, vec![1.0, 0.0]),
        Err(Error::InvalidQuery { .. })
    ));
    assert!(matches!(
        set(e, vec![1.0, f32::INFINITY, 0.0]),
        Err(Error::InvalidQuery { .. })
    ));
    assert!(matches!(
        set(e, vec![0.0, 0.0, 0.0]),
        Err(Error::InvalidQuery { .. })
    ));
    assert!(matches!(
        set(Eid::new(999), vec![1.0, 0.0, 0.0]),
        Err(Error::NotLive(_))
    ));
    let r = db.transact(TxOptions::default(), |tx| {
        tx.set_embedding(e, v("undefined-model"), &[1.0, 0.0, 0.0])
            .map(|_| ())
    });
    assert!(matches!(r, Err(Error::InvalidQuery { .. })), "{r:?}");
    // a retracted statement cannot be embedded
    let gone = fact(&db, "likes", "tea", &[0.0, 1.0, 0.0]);
    db.transact(TxOptions::default(), |tx| tx.retract(gone).map(|_| ()))
        .unwrap();
    assert!(matches!(
        set(gone, vec![0.0, 1.0, 0.0]),
        Err(Error::NotLive(_))
    ));
}

// @lat: [[tests#Vector Recall#Nearest Statements Rank First]]
#[test]
fn nearest_statements_rank_first_under_every_metric_and_encoding() {
    for (metric, encoding) in [
        (Metric::Cosine, VectorEncoding::F32),
        (Metric::Cosine, VectorEncoding::F16),
        (Metric::Dot, VectorEncoding::F32),
        (Metric::L2, VectorEncoding::F16),
    ] {
        let (_d, db) = open();
        define(
            &db,
            EmbeddingModel {
                dimensions: 3,
                metric,
                encoding,
            },
        );
        fact(&db, "livesIn", "paris", &[0.9, 0.1, 0.0]);
        fact(&db, "likes", "tea", &[0.0, 0.2, 0.9]);
        fact(&db, "worksAt", "acme", &[0.1, 0.9, 0.1]);
        let q = VectorQuery::new(m(&db), vec![1.0, 0.05, 0.0]);
        let r = db.now().vector_search(&q).unwrap();
        assert!(r.exact);
        assert_eq!(r.candidates, 3);
        let got = objects(&db, &db.now(), &q);
        assert_eq!(got[0], v("paris"), "{metric:?} {encoding:?}");
        assert_eq!(
            r.hits.iter().map(|h| h.rank).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(r
            .hits
            .windows(2)
            .all(|w| w[0].similarity >= w[1].similarity));
        let one = VectorQuery {
            limit: Some(1),
            ..q.clone()
        };
        assert_eq!(db.now().vector_search(&one).unwrap().hits.len(), 1);
    }
}

// @lat: [[tests#Vector Recall#Views Decide Visibility]]
#[test]
fn views_decide_which_embeddings_are_visible() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(2));
    let q = |db: &Db| VectorQuery::new(m(db), vec![1.0, 0.0]);
    // paris valid from 1 000 to 2 000, then retracted; lisbon live
    let mut paris = None;
    let r1 = db
        .transact(TxOptions::default(), |tx| {
            let e = tx
                .assert(
                    v("alice"),
                    v("livesIn"),
                    v("paris"),
                    Valid::between(1_000, 2_000),
                )?
                .eid();
            tx.set_embedding(e, model(), &[1.0, 0.0])?;
            paris = Some(e);
            Ok(())
        })
        .unwrap();
    fact(&db, "livesIn", "lisbon", &[0.6, 0.8]);
    db.transact(TxOptions::default(), |tx| {
        tx.retract(paris.unwrap()).map(|_| ())
    })
    .unwrap();
    // now: the retracted statement's vector is not visible
    assert_eq!(objects(&db, &db.now(), &q(&db)), vec![v("lisbon")]);
    // as of the first transaction: only paris existed
    assert_eq!(
        objects(&db, &db.as_of(TimeRef::Tx(r1.t.0)), &q(&db)),
        vec![v("paris")]
    );
    // history sees both, nearest first
    assert_eq!(
        objects(&db, &db.history(), &q(&db)),
        vec![v("paris"), v("lisbon")]
    );
    // valid time applies on top of transaction time
    assert_eq!(
        objects(&db, &db.history().valid_at(1_500), &q(&db)),
        vec![v("paris"), v("lisbon")]
    );
    assert_eq!(
        objects(&db, &db.history().valid_at(5_000), &q(&db)),
        vec![v("lisbon")]
    );
    // predicate and graph filters
    let tea = fact(&db, "likes", "tea", &[0.99, 0.01]);
    db.transact(TxOptions::default(), |tx| {
        tx.add_to_graph(tea, v("chat-7"), AssertOpts::default())
            .map(|_| ())
    })
    .unwrap();
    let likes = db.now().encode(&v("likes")).unwrap().unwrap();
    let only_likes = VectorQuery {
        predicates: Some(vec![likes]),
        ..q(&db)
    };
    assert_eq!(objects(&db, &db.now(), &only_likes), vec![v("tea")]);
    let chat = db.now().encode(&v("chat-7")).unwrap().unwrap();
    let in_chat = VectorQuery {
        graphs: Some(vec![chat]),
        ..q(&db)
    };
    assert_eq!(objects(&db, &db.now(), &in_chat), vec![v("tea")]);
    let none = VectorQuery {
        graphs: Some(vec![]),
        ..q(&db)
    };
    assert!(objects(&db, &db.now(), &none).is_empty());
}

// @lat: [[tests#Vector Recall#Writes Roll Back With The Transaction]]
#[test]
fn embedding_writes_roll_back_with_the_transaction() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(2));
    let e = fact(&db, "livesIn", "paris", &[1.0, 0.0]);
    let other = {
        let mut x = None;
        db.transact(TxOptions::default(), |tx| {
            x = Some(
                tx.assert(v("bob"), v("livesIn"), v("rome"), Valid::ALWAYS)?
                    .eid(),
            );
            Ok(())
        })
        .unwrap();
        x.unwrap()
    };
    // a failing transaction leaves no vector
    let r = db.transact(TxOptions::default(), |tx| {
        tx.set_embedding(other, model(), &[0.0, 1.0])?;
        Err(Error::custom("stop"))
    });
    assert!(r.is_err());
    // a dry run neither
    db.transact(
        TxOptions {
            dry_run: true,
            ..TxOptions::default()
        },
        |tx| tx.set_embedding(other, model(), &[0.0, 1.0]).map(|_| ()),
    )
    .unwrap();
    assert_eq!(db.now().unembedded(m(&db), None).unwrap().len(), 4); // rome + 3 model statements
                                                                     // a speculation sees its own vector, and leaves nothing behind
    let seen = db
        .with(
            |tx| tx.set_embedding(other, model(), &[0.0, 1.0]).map(|_| ()),
            |view| {
                let mm = view.encode(&model())?.unwrap();
                Ok(view
                    .vector_search(&VectorQuery::new(mm, vec![0.0, 1.0]))?
                    .hits[0]
                    .eid)
            },
        )
        .unwrap();
    assert_eq!(seen, other);
    let after = db
        .now()
        .vector_search(&VectorQuery::new(m(&db), vec![0.0, 1.0]))
        .unwrap();
    assert_eq!(after.hits.len(), 1);
    assert_eq!(after.hits[0].eid, e);
}

// @lat: [[tests#Vector Recall#Supersede Leaves A Backlog]]
#[test]
fn supersede_leaves_the_replacement_unembedded() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(2));
    let e = fact(&db, "worksAt", "acme", &[1.0, 0.0]);
    let mut new = None;
    db.transact(TxOptions::default(), |tx| {
        new = Some(tx.supersede(e, Patch::object(v("globex")))?);
        Ok(())
    })
    .unwrap();
    let new = new.unwrap();
    let mm = m(&db);
    // the old vector belongs to the retracted statement: invisible now
    assert!(db
        .now()
        .vector_search(&VectorQuery::new(mm, vec![1.0, 0.0]))
        .unwrap()
        .hits
        .is_empty());
    let worked = db.now().encode(&v("worksAt")).unwrap().unwrap();
    let backlog: Vec<Eid> = db
        .now()
        .unembedded(mm, None)
        .unwrap()
        .into_iter()
        .filter(|x| {
            db.now()
                .triples(None, Some(worked), None)
                .unwrap()
                .iter()
                .any(|t| t.eid == *x)
        })
        .collect();
    assert_eq!(backlog, vec![new]);
    assert_eq!(db.now().unembedded(mm, Some(1)).unwrap().len(), 1);
}

// @lat: [[tests#Vector Recall#Prefilter Reranks Exactly]]
#[test]
fn the_prefilter_reranks_its_candidates_exactly() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(8));
    let mut target = None;
    for i in 0..40u32 {
        let mut vec = vec![0.0f32; 8];
        for (k, x) in vec.iter_mut().enumerate() {
            *x = if (i >> (k % 5)) & 1 == 1 { 1.0 } else { -1.0 } + (k as f32) * 0.01;
        }
        let e = fact(&db, "note", &format!("n{i}"), &vec);
        if i == 13 {
            target = Some((e, vec));
        }
    }
    let (e, vec) = target.unwrap();
    let q = VectorQuery {
        prefilter: Some(5),
        limit: Some(3),
        ..VectorQuery::new(m(&db), vec.clone())
    };
    let r = db.now().vector_search(&q).unwrap();
    assert!(!r.exact);
    assert_eq!(r.candidates, 40);
    assert_eq!(r.hits[0].eid, e);
    assert!((r.hits[0].similarity - 1.0).abs() < 1e-5);
    let exact = db
        .now()
        .vector_search(&VectorQuery {
            limit: Some(3),
            ..VectorQuery::new(m(&db), vec)
        })
        .unwrap();
    assert_eq!(exact.hits[0].eid, e);
    assert!(exact.exact);
}

// @lat: [[tests#Vector Recall#Ties Break On Evidence]]
#[test]
fn equal_similarities_break_on_evidence_then_eid() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(2));
    let low = fact(&db, "livesIn", "paris", &[1.0, 0.0]);
    let high = fact(&db, "livesIn", "lisbon", &[1.0, 0.0]);
    let plain = fact(&db, "livesIn", "rome", &[1.0, 0.0]);
    db.transact(TxOptions::default(), |tx| {
        tx.assert(low, v("confidence"), Value::Double(0.2), Valid::ALWAYS)?;
        tx.assert(high, v("confidence"), Value::Double(0.9), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    let r = db
        .now()
        .vector_search(&VectorQuery::new(m(&db), vec![1.0, 0.0]))
        .unwrap();
    let order: Vec<Eid> = r.hits.iter().map(|h| h.eid).collect();
    assert_eq!(order, vec![high, low, plain]);
    assert_eq!(r.hits[0].evidence.confidence, Some(0.9));
    assert_eq!(r.hits[2].evidence.confidence, None);
    // with a limit of 1 the tie at the boundary is still decided by evidence
    let one = db
        .now()
        .vector_search(&VectorQuery {
            limit: Some(1),
            ..VectorQuery::new(m(&db), vec![1.0, 0.0])
        })
        .unwrap();
    assert_eq!(one.hits[0].eid, high);
}

// @lat: [[tests#Vector Recall#Embeddings Are Never Rewritten]]
#[test]
fn embeddings_cannot_be_rewritten_or_deleted_and_older_files_migrate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    {
        let db = Db::open(&path, OpenOptions::default()).unwrap();
        define(&db, EmbeddingModel::new(2));
        fact(&db, "livesIn", "paris", &[1.0, 0.0]);
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    for sql in ["DELETE FROM embedding", "UPDATE embedding SET vec = x'00'"] {
        let e = conn.execute_batch(sql).unwrap_err().to_string();
        assert!(e.contains("tiramemsu: embeddings are"), "{sql}: {e}");
    }
    // a format-4 file (no embedding table) migrates to format 5 on open
    conn.execute_batch(
        "DROP TRIGGER embedding_no_delete; DROP TRIGGER embedding_no_update; \
         DROP TABLE embedding; UPDATE meta SET value = 4 WHERE key = 'format_version';",
    )
    .unwrap();
    drop(conn);
    let db = Db::open(&path, OpenOptions::default()).unwrap();
    let format = db
        .read_sql("SELECT value FROM meta WHERE key = 'format_version'")
        .unwrap();
    assert_eq!(format[0][0].as_i64(), Some(5));
    let tables = db
        .read_sql("SELECT count(*) FROM sqlite_schema WHERE name = 'embedding'")
        .unwrap();
    assert_eq!(tables[0][0].as_i64(), Some(1));
    // the definition survived (it is graph data); vectors are re-supplied
    assert!(db
        .now()
        .vector_search(&VectorQuery::new(m(&db), vec![1.0, 0.0]))
        .unwrap()
        .hits
        .is_empty());
}

// @lat: [[tests#Vector Recall#Hybrid Recall Fuses Both Channels]]
#[test]
fn hybrid_recall_fuses_text_and_vectors_and_expands() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(
        dir.path().join("m.db"),
        OpenOptions {
            text_index: true,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    define(&db, EmbeddingModel::new(2));
    let note = |text: &str, vec: &[f32]| {
        let mut eid = None;
        db.transact(TxOptions::default(), |tx| {
            let e = tx
                .assert(v("trip"), v("note"), Value::str(text), Valid::ALWAYS)?
                .eid();
            tx.set_embedding(e, model(), vec)?;
            eid = Some(e);
            Ok(())
        })
        .unwrap();
        eid.unwrap()
    };
    let both = note("lisbon offsite in june", &[1.0, 0.0]);
    let text_only = note("lisbon tram tickets", &[0.0, 1.0]);
    let vector_only = note("portugal team week", &[0.95, 0.05]);
    db.transact(TxOptions::default(), |tx| {
        tx.assert(both, v("source"), Value::str("chat-12"), Valid::ALWAYS)
            .map(|_| ())
    })
    .unwrap();
    let q = RecallQuery {
        text: Some(TextQuery::new("lisbon")),
        vector: Some(VectorQuery::new(m(&db), vec![1.0, 0.0])),
        ..RecallQuery::default()
    };
    let hits = db.now().recall(&q).unwrap();
    assert_eq!(hits[0].eid, both, "found by both channels");
    assert!(hits[0].text_rank.is_some() && hits[0].vector_rank.is_some());
    let ids: Vec<Eid> = hits.iter().map(|h| h.eid).collect();
    assert!(ids.contains(&text_only) && ids.contains(&vector_only));
    assert!(hits.iter().all(|h| h.via.is_none()));
    assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
    // expansion adds the source layer of the top hit, marked with `via`
    let expanded = db
        .now()
        .recall(&RecallQuery {
            limit: Some(1),
            expand: 1,
            ..q.clone()
        })
        .unwrap();
    assert_eq!(expanded[0].eid, both);
    let layer = expanded
        .iter()
        .find(|h| h.via == Some(both) && h.s == both.oid())
        .expect("the source layer");
    assert_eq!(db.now().decode(layer.p).unwrap(), v("source"));
    assert_eq!(layer.score, 0.0);
    // a vector-only recall works without the text index
    let vec_only = db
        .now()
        .recall(&RecallQuery {
            vector: Some(VectorQuery::new(m(&db), vec![0.0, 1.0])),
            ..RecallQuery::default()
        })
        .unwrap();
    assert_eq!(vec_only[0].eid, text_only);
    assert!(matches!(
        db.now().recall(&RecallQuery::default()),
        Err(Error::InvalidQuery { .. })
    ));
}

// @lat: [[tests#Vector Recall#SPARQL And Cypher Share The Search]]
#[cfg(all(feature = "sparql", feature = "cypher"))]
#[test]
fn sparql_and_cypher_run_the_same_search_as_the_rust_api() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(2));
    let paris = fact(&db, "livesIn", "paris", &[1.0, 0.0]);
    let tea = fact(&db, "likes", "tea", &[0.0, 1.0]);
    let r1 = db
        .events_since(0)
        .unwrap()
        .iter()
        .map(|e| e.t.0)
        .max()
        .unwrap();
    let lisbon = fact(&db, "livesIn", "lisbon", &[0.8, 0.6]);
    db.transact(TxOptions::default(), |tx| {
        tx.add_to_graph(tea, v("chat"), AssertOpts::default())
            .map(|_| ())
    })
    .unwrap();
    let api = db
        .now()
        .vector_search(&VectorQuery::new(m(&db), vec![1.0, 0.1]))
        .unwrap()
        .hits;
    assert_eq!(
        api.iter().map(|h| h.eid).collect::<Vec<_>>(),
        vec![paris, lisbon, tea]
    );
    // SPARQL
    let q = "SELECT ?e ?sim ?rank ?o WHERE { \
               ?e tm:vectorMatch \"[1.0, 0.1]\" ; tm:vectorModel v:toy-embedder ; \
                  tm:vectorSimilarity ?sim ; tm:vectorRank ?rank . \
               ?s ?p ?o ~ ?e } ORDER BY ?rank";
    let res = db.now().sparql(q).unwrap();
    let sol = res.solutions().unwrap();
    let col = |n: &str| sol.vars.iter().position(|x| x == n).unwrap();
    assert_eq!(sol.rows.len(), 3);
    for (row, hit) in sol.rows.iter().zip(&api) {
        assert_eq!(row[col("e")], Some(Value::Stmt(hit.eid)));
        assert_eq!(row[col("sim")], Some(Value::Double(hit.similarity)));
        assert_eq!(row[col("rank")], Some(Value::Int(hit.rank as i64)));
    }
    let top = db
        .now()
        .sparql("SELECT ?e WHERE { ?e tm:vectorMatch \"[1, 0.1]\" ; tm:vectorModel v:toy-embedder ; tm:vectorLimit 1 }")
        .unwrap();
    assert_eq!(
        top.solutions().unwrap().rows,
        vec![vec![Some(Value::Stmt(paris))]]
    );
    let in_chat = db
        .now()
        .sparql("SELECT ?e WHERE { GRAPH v:chat { ?e tm:vectorMatch \"[1, 0]\" ; tm:vectorModel v:toy-embedder } }")
        .unwrap();
    assert_eq!(
        in_chat.solutions().unwrap().rows,
        vec![vec![Some(Value::Stmt(tea))]]
    );
    let before = db
        .now()
        .sparql(&format!(
            "SELECT ?e WHERE {{ SERVICE <urn:tiramemsu:tm:asOf/{r1}> {{ ?e tm:vectorMatch \"[0, 1]\" ; tm:vectorModel v:toy-embedder }} }}"
        ))
        .unwrap();
    assert_eq!(
        before.solutions().unwrap().rows,
        vec![vec![Some(Value::Stmt(tea))], vec![Some(Value::Stmt(paris))]]
    );
    for bad in [
        "SELECT ?e WHERE { ?e tm:vectorMatch \"[1, 0]\" }",
        "SELECT ?e WHERE { ?e tm:vectorModel v:toy-embedder }",
        "SELECT ?e WHERE { ?e tm:vectorMatch \"1, 0\" ; tm:vectorModel v:toy-embedder }",
        "SELECT ?e WHERE { GRAPH ?g { ?e tm:vectorMatch \"[1, 0]\" ; tm:vectorModel v:toy-embedder } }",
    ] {
        assert!(
            matches!(db.now().sparql(bad), Err(Error::InvalidQuery { .. })),
            "{bad}"
        );
    }
    // Cypher
    let none = CypherParams::default();
    let c = db
        .now()
        .cypher(
            "CALL tiramemsu.vector.search('toy-embedder', [1.0, 0.1]) \
             YIELD statement, object, similarity, rank RETURN statement, object, similarity, rank",
            &none,
        )
        .unwrap();
    assert_eq!(c.rows.len(), 3);
    for (row, hit) in c.rows.iter().zip(&api) {
        match &row[0] {
            CypherValue::Node(n) => {
                assert_eq!(n.element_id, format!("urn:tiramemsu:stmt:{}", hit.eid.n()))
            }
            other => panic!("statement is {other:?}"),
        }
        assert_eq!(row[2], CypherValue::Float(hit.similarity));
        assert_eq!(row[3], CypherValue::Integer(hit.rank as i64));
    }
    let opts = db
        .now()
        .cypher(
            "CALL tiramemsu.vector.search('toy-embedder', [1, 0], {limit: 5, graphs: ['chat']}) \
             YIELD predicate RETURN predicate",
            &none,
        )
        .unwrap();
    assert_eq!(opts.rows, vec![vec![CypherValue::String("likes".into())]]);
    assert!(db
        .now()
        .cypher("CALL tiramemsu.vector.search('toy-embedder', ['x'])", &none)
        .is_err());
}

// @lat: [[tests#Vector Recall#Past Views See Past Embeddings]]
#[test]
fn an_as_of_view_sees_only_the_embeddings_stored_by_then() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(2));
    let mut e = None;
    let t1 = db
        .transact(TxOptions::default(), |tx| {
            e = Some(
                tx.assert(v("alice"), v("livesIn"), v("paris"), Valid::ALWAYS)?
                    .eid(),
            );
            Ok(())
        })
        .unwrap()
        .t;
    let e = e.unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.set_embedding(e, model(), &[1.0, 0.0]).map(|_| ())
    })
    .unwrap();
    let q = VectorQuery::new(m(&db), vec![1.0, 0.0]);
    // the statement was live at t1, but its vector arrived later
    let past = db.as_of(TimeRef::Tx(t1.0));
    assert!(past.vector_search(&q).unwrap().hits.is_empty());
    assert!(past.unembedded(m(&db), None).unwrap().contains(&e));
    assert_eq!(db.now().vector_search(&q).unwrap().hits[0].eid, e);
    assert!(!db.now().unembedded(m(&db), None).unwrap().contains(&e));
}

// @lat: [[tests#Vector Recall#Budgets Bound A Search]]
#[test]
fn a_budget_bounds_a_vector_search() {
    let (_d, db) = open();
    define(&db, EmbeddingModel::new(2));
    for i in 0..5 {
        fact(&db, "note", &format!("n{i}"), &[1.0, i as f32]);
    }
    let q = VectorQuery::new(m(&db), vec![1.0, 0.0]);
    let rows = QueryBudget {
        max_rows: Some(2),
        ..QueryBudget::default()
    };
    assert!(matches!(
        db.now().with_budget(&rows).vector_search(&q),
        Err(Error::ResultLimitExceeded { .. })
    ));
    let cancel = CancelToken::new();
    cancel.cancel();
    let cancelled = QueryBudget {
        cancel: Some(cancel),
        ..QueryBudget::default()
    };
    assert!(matches!(
        db.now().with_budget(&cancelled).vector_search(&q),
        Err(Error::Cancelled)
    ));
    let roomy = QueryBudget {
        max_rows: Some(100),
        ..QueryBudget::default()
    };
    assert_eq!(
        db.now()
            .with_budget(&roomy)
            .vector_search(&q)
            .unwrap()
            .hits
            .len(),
        5
    );
}
