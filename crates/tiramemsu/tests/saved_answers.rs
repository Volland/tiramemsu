//! Saved answers and conservative invalidation (OpenSpec change
//! `add-saved-answer-invalidation`): stored queries with their parameters, view
//! and result, event-driven `recheck` and `stale` marks, replayable checkpoints,
//! and refreshes that only clear a mark when they succeed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("saved.db");
    (d, p)
}

fn open(p: &Path) -> Db {
    Db::open(p, OpenOptions::default()).unwrap()
}

fn assert_one(db: &Db, s: &str, p: &str, o: Value) -> Eid {
    let mut eid = None;
    db.transact(TxOptions::default(), |tx| {
        eid = Some(tx.assert(v(s), v(p), o, Valid::ALWAYS)?.eid());
        Ok(())
    })
    .unwrap();
    eid.unwrap()
}

fn retract(db: &Db, eid: Eid) {
    db.transact(TxOptions::default(), |tx| tx.retract(eid).map(|_| ()))
        .unwrap();
}

/// The graph rows, the counters and the event log: what saved answers must
/// never change.
fn history(db: &Db) -> (Vec<Vec<SqlValue>>, Vec<Event>) {
    let mut rows = db
        .read_sql("SELECT key, value FROM meta WHERE key <> 'format_version' ORDER BY key")
        .unwrap();
    rows.extend(db.read_sql("SELECT count(*) FROM tx").unwrap());
    rows.extend(db.read_sql("SELECT count(*) FROM triple").unwrap());
    (rows, db.events_since(0).unwrap())
}

const EMPLOYER: &str = "SELECT ?o WHERE { v:alice v:worksAt ?o }";

// @lat: [[tests#Saved Answers#Save A Parameterized Query]]
#[test]
fn a_saved_parameterized_query_recovers_its_parameters_and_view() {
    let (_d, p) = tmp();
    let first = {
        let db = open(&p);
        db.transact(TxOptions::default(), |tx| {
            tx.set_vocab("urn:tiramemsu:v:")?;
            tx.assert(v("alice"), v("name"), Value::str("Alice"), Valid::ALWAYS)?;
            tx.assert(v("alice"), v("age"), Value::Int(41), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
        let mut params = CypherParams::new();
        params.insert("name".into(), CypherValue::String("Alice".into()));
        params.insert("min".into(), CypherValue::Integer(40));
        params.insert(
            "since".into(),
            CypherValue::DateTime {
                ms: 1_700_000_000_000,
                tz: 60,
            },
        );
        params.insert(
            "tags".into(),
            CypherValue::List(vec![CypherValue::Float(0.5), CypherValue::Null]),
        );
        let view = ViewSpec::as_of(TimeRef::Tx(1)).valid_at(1_000);
        let q = SavedQuery::cypher(
            "MATCH (p {name: $name}) WHERE p.age >= $min RETURN p.age AS age",
            params,
        )
        .on(view);
        let a = db.save_answer("age", &q).unwrap();
        assert_eq!(a.query, q);
        assert_eq!(a.revision, 1);
        assert_eq!(a.result["rows"], serde_json::json!([[41]]));
        assert_eq!(a.vocab.as_deref(), Some("urn:tiramemsu:v:"));
        a
    };
    // a new handle (and a different default view) recovers the same record
    let db = open(&p);
    db.transact(TxOptions::default(), |tx| {
        tx.set_vocab("urn:example:other#")
    })
    .unwrap();
    let back = db.saved_answer("age").unwrap().unwrap();
    assert_eq!(back, first);
    assert_eq!(
        back.query.view,
        ViewSpec::as_of(TimeRef::Tx(1)).valid_at(1_000)
    );
    // the refresh runs with the stored parameters, view and settings, not the
    // current @vocab
    let again = db.refresh_answer("age").unwrap();
    assert_eq!(again.result, first.result);
    assert_eq!(again.revision, 2);
    assert_eq!(again.vocab.as_deref(), Some("urn:tiramemsu:v:"));
    assert_eq!(db.saved_answers().unwrap().len(), 1);
    // SPARQL text has no parameters: refused before anything is stored
    let mut params = CypherParams::new();
    params.insert("x".into(), CypherValue::Integer(1));
    let mut q = SavedQuery::sparql(EMPLOYER);
    q.params = params;
    assert!(matches!(
        db.save_answer("bad", &q),
        Err(Error::Unsupported { .. })
    ));
    assert!(db.saved_answer("bad").unwrap().is_none());
}

// @lat: [[tests#Saved Answers#Negative Condition Requests Recheck]]
#[test]
fn an_insert_matching_not_exists_requests_a_recheck() {
    let (_d, p) = tmp();
    let db = open(&p);
    assert_one(&db, "alice", "type", v("Person"));
    let q = SavedQuery::sparql(
        "SELECT ?p WHERE { ?p v:type v:Person FILTER NOT EXISTS { ?p v:worksAt ?c } }",
    );
    let a = db.save_answer("unemployed", &q).unwrap();
    assert_eq!(a.solutions().unwrap().unwrap().rows.len(), 1);
    assert!(a.coverage.contains(&CoverageReason::NegativePattern));
    assert!(a.coverage.contains(&CoverageReason::MutableView));
    // the supporting statement stays live, yet the answer is no longer trusted
    let hired = assert_one(&db, "alice", "worksAt", v("acme"));
    let marks = db.check_saved_answers().unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].status, AnswerStatus::Recheck);
    assert_eq!(marks[0].cause, InvalidationCause::Insertion);
    assert_eq!(marks[0].event.unwrap().eid, hired);
    let a = db.saved_answer("unemployed").unwrap().unwrap();
    assert_eq!(a.status, AnswerStatus::Recheck);
    // the re-run shows the row is gone
    let a = db.refresh_answer("unemployed").unwrap();
    assert!(a.is_fresh());
    assert!(a.solutions().unwrap().unwrap().rows.is_empty());
}

// @lat: [[tests#Saved Answers#Additional Matching Row Requests Recheck]]
#[test]
fn a_new_matching_row_requests_a_recheck_with_live_support() {
    let (_d, p) = tmp();
    let db = open(&p);
    let acme = assert_one(&db, "alice", "worksAt", v("acme"));
    let a = db
        .save_answer("employer", &SavedQuery::sparql(EMPLOYER))
        .unwrap();
    assert_eq!(a.dependencies, [acme]);
    assert_eq!(a.checkpoint, TxId(1));
    let globex = assert_one(&db, "alice", "worksAt", v("globex"));
    let marks = db.check_saved_answers().unwrap();
    assert_eq!(marks.len(), 1);
    let m = &marks[0];
    assert_eq!(
        (m.status, m.cause, m.t),
        (
            AnswerStatus::Recheck,
            InvalidationCause::Insertion,
            Some(TxId(2))
        )
    );
    assert_eq!(m.event.unwrap().eid, globex);
    assert_eq!(m.event.unwrap().op, Op::Assert);
    // every supporting eid is still live
    assert!(db
        .now()
        .triples(None, None, None)
        .unwrap()
        .iter()
        .any(|t| t.eid == acme));
    let a = db.saved_answer("employer").unwrap().unwrap();
    assert_eq!(a.invalidation.as_ref(), Some(m));
    assert_eq!(a.checkpoint, TxId(1));
    assert_eq!(a.cursor, TxId(2));
    let a = db.refresh_answer("employer").unwrap();
    assert_eq!(a.solutions().unwrap().unwrap().rows.len(), 2);
    assert_eq!(a.dependencies, [acme, globex]);
    assert_eq!(a.checkpoint, TxId(2));
}

// @lat: [[tests#Saved Answers#Retracted Support Marks Stale]]
#[test]
fn retracting_or_superseding_support_marks_stale_with_the_event() {
    let (_d, p) = tmp();
    let db = open(&p);
    let acme = assert_one(&db, "alice", "worksAt", v("acme"));
    let paris = assert_one(&db, "acme", "in", v("paris"));
    db.save_answer("employer", &SavedQuery::sparql(EMPLOYER))
        .unwrap();
    db.save_answer(
        "city",
        &SavedQuery::sparql("SELECT ?c WHERE { v:alice v:worksAt ?o . ?o v:in ?c }"),
    )
    .unwrap();
    // an unrelated insertion first: both become recheck
    assert_one(&db, "bob", "worksAt", v("initech"));
    assert_eq!(db.check_saved_answers().unwrap().len(), 2);
    // an explicit retraction upgrades `employer` (and `city`) to stale
    retract(&db, acme);
    let marks = db.check_saved_answers().unwrap();
    assert_eq!(marks.len(), 2);
    for m in &marks {
        assert_eq!(m.status, AnswerStatus::Stale);
        assert_eq!(m.cause, InvalidationCause::SupportRetracted);
        let ev = m.event.unwrap();
        assert_eq!(
            (ev.eid, ev.op, ev.kind),
            (acme, Op::Retract, Some(RetKind::Explicit))
        );
    }
    // a supersede of a cited statement is reported with its retraction kind
    let loc = db
        .save_answer(
            "location",
            &SavedQuery::sparql("SELECT ?c WHERE { v:acme v:in ?c }"),
        )
        .unwrap();
    assert_eq!(loc.dependencies, [paris]);
    db.transact(TxOptions::default(), |tx| {
        tx.supersede(
            paris,
            Patch {
                o: Some(v("berlin")),
                ..Patch::default()
            },
        )
        .map(|_| ())
    })
    .unwrap();
    let marks = db.check_saved_answers().unwrap();
    // `city` and `employer` were already stale: only `location` is reported
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].name, "location");
    assert_eq!(marks[0].status, AnswerStatus::Stale);
    let ev = marks[0].event.unwrap();
    assert_eq!((ev.eid, ev.kind), (paris, Some(RetKind::Supersede)));
    let city = db.saved_answer("city").unwrap().unwrap();
    assert_eq!(city.invalidation.unwrap().event.unwrap().eid, acme);
}

// @lat: [[tests#Saved Answers#Restart Replays Without Duplicates]]
#[test]
fn processing_resumes_from_the_saved_cursor_without_loss_or_duplicates() {
    let (d, p) = tmp();
    let acme;
    {
        let db = open(&p);
        acme = assert_one(&db, "alice", "worksAt", v("acme"));
        db.save_answer("employer", &SavedQuery::sparql(EMPLOYER))
            .unwrap();
        assert_one(&db, "bob", "worksAt", v("initech"));
        let marks = db.check_saved_answers().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].cause, InvalidationCause::Insertion);
        // processed once: a second run reports nothing
        assert!(db.check_saved_answers().unwrap().is_empty());
        retract(&db, acme);
    }
    // a copy of the file whose consumer never ran again
    let copy = d.path().join("copy.db");
    std::fs::copy(&p, &copy).unwrap();
    let wal = d.path().join("saved.db-wal");
    if wal.exists() {
        std::fs::copy(&wal, d.path().join("copy.db-wal")).unwrap();
    }
    {
        // restart: the retraction after the cursor is found exactly once
        let db = open(&p);
        let marks = db.check_saved_answers().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].status, AnswerStatus::Stale);
        assert_eq!(marks[0].event.unwrap().eid, acme);
        assert!(db.check_saved_answers().unwrap().is_empty());
        let a = db.saved_answer("employer").unwrap().unwrap();
        assert_eq!(a.cursor, TxId(3));
        assert_eq!(a.checkpoint, TxId(1));
    }
    {
        let db = open(&p);
        assert!(db.check_saved_answers().unwrap().is_empty());
    }
    // replaying the same events from the older cursor reaches the same marks
    let replayed = open(&copy);
    replayed.check_saved_answers().unwrap();
    assert_eq!(
        replayed.saved_answer("employer").unwrap(),
        open(&p).saved_answer("employer").unwrap()
    );
}

// @lat: [[tests#Saved Answers#Failed Refresh Keeps Stale]]
#[test]
fn a_failed_refresh_keeps_the_old_result_stale_and_the_checkpoint() {
    let (_d, p) = tmp();
    let db = open(&p);
    let acme = assert_one(&db, "alice", "worksAt", v("acme"));
    let saved = db
        .save_answer("employer", &SavedQuery::sparql(EMPLOYER))
        .unwrap();
    retract(&db, acme);
    assert_eq!(db.check_saved_answers().unwrap().len(), 1);
    let stale = db.saved_answer("employer").unwrap().unwrap();
    let token = CancelToken::new();
    token.cancel();
    let budget = QueryBudget {
        cancel: Some(token),
        ..Default::default()
    };
    let r = db.refresh_answer_with("employer", Some(&budget));
    assert!(matches!(r, Err(Error::Cancelled)));
    let after = db.saved_answer("employer").unwrap().unwrap();
    assert_eq!(after.status, AnswerStatus::Stale);
    assert_eq!(after.checkpoint, saved.checkpoint);
    assert_eq!(after.result, saved.result);
    assert_eq!(after.revision, 1);
    assert_eq!(after.invalidation, stale.invalidation);
    assert!(after.error.as_deref().unwrap().contains("cancel"));
    // acknowledging the mark is not enough: checking again keeps it stale
    assert!(db.check_saved_answers().unwrap().is_empty());
    assert_eq!(
        db.saved_answer("employer").unwrap().unwrap().status,
        AnswerStatus::Stale
    );
    // a successful re-run clears the mark and the error, and advances
    let ok = db.refresh_answer("employer").unwrap();
    assert!(ok.is_fresh());
    assert_eq!(ok.error, None);
    assert_eq!(ok.checkpoint, TxId(2));
    assert_eq!(ok.revision, 2);
    assert!(ok.solutions().unwrap().unwrap().rows.is_empty());
    // a missing answer is a typed error
    assert!(matches!(
        db.refresh_answer("nope"),
        Err(Error::SavedAnswerNotFound { .. })
    ));
}

// @lat: [[tests#Saved Answers#Fixed Historical Views Stay Fresh]]
#[test]
fn fixed_historical_views_stay_fresh_unless_they_read_the_clock() {
    let (_d, p) = tmp();
    let clock = Arc::new(ManualClock::new(1_000_000));
    let db = Db::open(
        &p,
        OpenOptions {
            clock: clock.clone(),
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let acme = assert_one(&db, "alice", "worksAt", v("acme"));
    let at1 = ViewSpec::as_of(TimeRef::Tx(1));
    let fixed = db
        .save_answer("then", &SavedQuery::sparql(EMPLOYER).on(at1))
        .unwrap();
    assert!(fixed.coverage.is_empty());
    let by_instant = db
        .save_answer(
            "then-instant",
            &SavedQuery::sparql(EMPLOYER).on(ViewSpec::as_of(TimeRef::Instant(1_000_000))),
        )
        .unwrap();
    assert!(by_instant.coverage.is_empty());
    // an as-of point after the head can still change
    let future = db
        .save_answer(
            "future",
            &SavedQuery::sparql(EMPLOYER).on(ViewSpec::as_of(TimeRef::Tx(5))),
        )
        .unwrap();
    assert_eq!(future.coverage, [CoverageReason::MutableView]);
    let clocked = db
        .save_answer(
            "clocked",
            &SavedQuery::sparql("SELECT ?o (NOW() AS ?at) WHERE { v:alice v:worksAt ?o }").on(at1),
        )
        .unwrap();
    assert_eq!(clocked.coverage, [CoverageReason::Clock]);
    let cypher_clock = db
        .save_answer(
            "cypher-clocked",
            &SavedQuery::cypher("RETURN datetime() AS now", CypherParams::new()).on(at1),
        )
        .unwrap();
    assert!(cypher_clock.coverage.contains(&CoverageReason::Clock));
    // history moves on: the cited statement is retracted, another one inserted
    retract(&db, acme);
    assert_one(&db, "alice", "worksAt", v("globex"));
    clock.advance(60_000);
    let marks = db.check_saved_answers().unwrap();
    let mut names: Vec<_> = marks.iter().map(|m| m.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["clocked", "cypher-clocked", "future"]);
    for m in &marks {
        let want = if m.name == "future" {
            // the as-of point not yet reached sees the retraction of its support
            (AnswerStatus::Stale, InvalidationCause::SupportRetracted)
        } else {
            (AnswerStatus::Recheck, InvalidationCause::Clock)
        };
        assert_eq!((m.status, m.cause), want);
    }
    for name in ["then", "then-instant"] {
        let a = db.saved_answer(name).unwrap().unwrap();
        assert!(a.is_fresh(), "{name}");
        assert_eq!(a.cursor, TxId(3));
        assert_eq!(a.solutions().unwrap().unwrap().rows.len(), 1);
    }
}

// @lat: [[tests#Saved Answers#Coverage Reasons Are Explicit]]
#[test]
fn coverage_reasons_name_what_provenance_cannot_prove() {
    let (_d, p) = tmp();
    let db = open(&p);
    assert_one(&db, "alice", "worksAt", v("acme"));
    assert_one(&db, "acme", "in", v("paris"));
    let cov = |name: &str, q: SavedQuery| db.save_answer(name, &q).unwrap().coverage;
    use CoverageReason::*;
    assert_eq!(
        cov(
            "path",
            SavedQuery::sparql("SELECT ?c WHERE { v:alice (v:worksAt/v:in)+ ?c }")
        ),
        [MutableView, RecursivePath]
    );
    assert_eq!(
        cov(
            "exists",
            SavedQuery::sparql("SELECT ?p WHERE { ?p v:worksAt ?o FILTER EXISTS { ?o v:in ?c } }")
        ),
        [MutableView, ExistsPattern]
    );
    assert_eq!(
        cov(
            "minus",
            SavedQuery::sparql("SELECT ?p WHERE { ?p v:worksAt ?o MINUS { ?p v:in ?c } }")
        ),
        [MutableView, NegativePattern]
    );
    assert_eq!(
        cov(
            "virtual",
            SavedQuery::sparql("SELECT ?t WHERE { ?r sys:subject v:alice ; tm:txAdded ?t }")
        ),
        [MutableView, VirtualPredicate]
    );
    assert_eq!(
        cov(
            "ask",
            SavedQuery::sparql("ASK { v:alice v:worksAt v:acme }")
        ),
        [MutableView, NoProvenance]
    );
    assert_eq!(
        db.saved_answer("ask").unwrap().unwrap().boolean(),
        Some(true)
    );
    assert_eq!(
        cov(
            "cypher",
            SavedQuery::cypher(
                "MATCH (a)-[:worksAt]->(o) RETURN count(*) AS n",
                CypherParams::new()
            )
        ),
        [MutableView, NoProvenance]
    );
    // a returned node or a property read carries volatile values
    assert_eq!(
        cov(
            "cypher-node",
            SavedQuery::cypher("MATCH (a)-[:worksAt]->(o) RETURN o", CypherParams::new())
        ),
        [MutableView, NoProvenance, Volatile]
    );
    // updates and CONSTRUCT are not answers
    for text in [
        "INSERT DATA { v:a v:p v:b }",
        "CONSTRUCT { ?s v:p ?o } WHERE { ?s v:worksAt ?o }",
    ] {
        assert!(matches!(
            db.save_answer("no", &SavedQuery::sparql(text)),
            Err(Error::Unsupported { .. })
        ));
    }
    assert!(matches!(
        db.save_answer("", &SavedQuery::sparql(EMPLOYER)),
        Err(Error::InvalidQuery { .. })
    ));
}

// @lat: [[tests#Saved Answers#Saved Answers Never Touch History]]
#[test]
fn saving_checking_and_refreshing_leave_graph_history_alone() {
    let (_d, p) = tmp();
    let db = open(&p);
    let acme = assert_one(&db, "alice", "worksAt", v("acme"));
    // a transaction with only a volatile change still requests a recheck
    db.save_answer("employer", &SavedQuery::sparql(EMPLOYER))
        .unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.set_volatile(v("alice"), v("mood"), Value::str("calm"))
    })
    .unwrap();
    let before = history(&db);
    let marks = db.check_saved_answers().unwrap();
    assert_eq!(marks[0].cause, InvalidationCause::Transaction);
    assert_eq!((marks[0].t, marks[0].event), (Some(TxId(2)), None));
    db.refresh_answer("employer").unwrap();
    db.save_answer("other", &SavedQuery::sparql(EMPLOYER))
        .unwrap();
    assert!(db.delete_saved_answer("other").unwrap());
    assert!(!db.delete_saved_answer("other").unwrap());
    assert_eq!(history(&db), before);
    // saving is refused while a bulk import holds the write lease
    let import = db.bulk_import().unwrap();
    assert!(matches!(
        db.save_answer("x", &SavedQuery::sparql(EMPLOYER)),
        Err(Error::ImportInProgress)
    ));
    assert!(matches!(
        db.check_saved_answers(),
        Err(Error::ImportInProgress)
    ));
    import.cancel();
    // and inside a transaction
    let r = db.transact(TxOptions::default(), |_| {
        db.check_saved_answers().map(|_| ())
    });
    assert!(matches!(r, Err(Error::Reentrant)));
    retract(&db, acme);
    assert_eq!(db.check_saved_answers().unwrap().len(), 1);
}

// @lat: [[tests#Saved Answers#Migration To Format 3 Keeps Every Row]]
#[test]
fn migrating_a_format_2_file_adds_the_tables_and_keeps_every_row() {
    use tm_core::storage;
    let (_d, p) = tmp();
    {
        let exec = storage::open_with(
            &RusqliteHost::new(),
            &p,
            &HostOptions::default(),
            storage::migrate::MIGRATIONS,
            2,
        )
        .unwrap();
        let mut store = tm_core::Store::from_executor(exec, &p, tm_core::StoreOptions::default());
        store
            .transact(TxOptions::default(), |tx| {
                let e = tx.assert(v("ann"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
                tx.assert(v("bob"), v("worksAt"), v("initech"), Valid::ALWAYS)?;
                tx.retract(e.eid()).map(|_| ())
            })
            .unwrap();
    }
    let raw = |p: &Path, sql: &str| -> Vec<String> {
        let c = rusqlite::Connection::open(p).unwrap();
        let mut st = c.prepare(sql).unwrap();
        let n = st.column_count();
        st.query_map([], |r| {
            Ok((0..n)
                .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join(","))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    };
    let graph = |p: &Path| {
        let mut out = Vec::new();
        for t in ["triple", "term", "tx"] {
            out.extend(raw(p, &format!("SELECT * FROM {t} ORDER BY 1")));
        }
        out
    };
    let version = "SELECT value FROM meta WHERE key = 'format_version'";
    assert_eq!(raw(&p, version), ["Integer(2)"]);
    assert!(raw(
        &p,
        "SELECT name FROM sqlite_schema WHERE name = 'saved_answer'"
    )
    .is_empty());
    let before = graph(&p);
    let db = open(&p);
    assert_eq!(storage::FORMAT_VERSION, 3);
    assert_eq!(raw(&p, version), ["Integer(3)"]);
    assert_eq!(graph(&p), before);
    assert!(db.saved_answers().unwrap().is_empty());
    let a = db
        .save_answer(
            "employer",
            &SavedQuery::sparql("SELECT ?o WHERE { v:bob v:worksAt ?o }"),
        )
        .unwrap();
    assert_eq!(a.dependencies.len(), 1);
    assert_eq!(graph(&p), before);
}
