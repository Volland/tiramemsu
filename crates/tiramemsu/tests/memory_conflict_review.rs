//! Spec `memory-conflict-review`: conflict inspection on a view and noncommitting
//! bundle import previews, through the facade.

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn open(dir: &tempfile::TempDir, name: &str) -> Db {
    Db::open(dir.path().join(name), OpenOptions::default()).unwrap()
}

fn id(db: &Db, x: &Value) -> ObjectId {
    db.now().encode(x).unwrap().unwrap()
}

/// Everything a write could change, read through the public API.
fn footprint(db: &Db) -> (usize, usize, usize) {
    (
        db.history().triples(None, None, None).unwrap().len(),
        db.events_since(0).unwrap().len(),
        db.now().triples(None, None, None).unwrap().len(),
    )
}

// @lat: [[tests#Memory Conflict Review#Disjoint Valid Times Are Not Conflicts]]
#[test]
fn disjoint_valid_times_are_not_reported_and_overlaps_are() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    db.transact(TxOptions::default(), |tx| {
        // bob changed jobs: [0, 10) then [10, ∞), half open, no overlap
        tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::between(0, 10))?;
        tx.assert(v("bob"), v("worksAt"), v("initech"), Valid::from(10))?;
        // alice: acme from 0, initech during [5, 8) and again [20, 30)
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::between(0, 25))?;
        tx.assert(v("alice"), v("worksAt"), v("initech"), Valid::between(5, 8))?;
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("initech"),
            Valid::between(20, 30),
        )?;
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("globex"),
            Valid::between(40, 50),
        )?;
        Ok(())
    })
    .unwrap();
    let all = db.now().conflicts(&ConflictQuery::default()).unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    let c = &all[0];
    assert_eq!(c.s, id(&db, &v("alice")));
    assert_eq!(c.p, id(&db, &v("worksAt")));
    assert_eq!(
        c.overlaps,
        vec![Valid::between(5, 8), Valid::between(20, 25)]
    );
    // globex never overlaps anything and is not part of the conflict
    let objects: Vec<_> = c.values.iter().map(|x| x.o).collect();
    assert_eq!(objects, vec![id(&db, &v("acme")), id(&db, &v("initech"))]);
    assert_eq!(c.values[1].statements.len(), 2);
    assert!(!c.declared_many);
    // a valid-time view narrows the question to one instant
    let at = |ms| {
        db.now()
            .valid_at(ms)
            .conflicts(&ConflictQuery::default())
            .unwrap()
    };
    assert_eq!(at(6).len(), 1);
    assert!(at(12).is_empty());
    assert!(at(45).is_empty());
}

// @lat: [[tests#Memory Conflict Review#Parallel Equal Objects Are Support]]
#[test]
fn parallel_statements_with_the_same_object_are_support() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    let r = db
        .transact(TxOptions::default(), |tx| {
            // two sources say the same thing: parallel eids, one value
            let a = tx.create(v("carol"), v("livesIn"), v("lisbon"), Valid::ALWAYS)?;
            let b = tx.create(v("carol"), v("livesIn"), v("lisbon"), Valid::from(5))?;
            tx.assert(a, v("source"), Value::str("email"), Valid::ALWAYS)?;
            tx.assert(b, v("source"), Value::str("chat"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    assert!(db
        .now()
        .conflicts(&ConflictQuery::default())
        .unwrap()
        .is_empty());
    // a third, different object makes a conflict with one supported value
    let r2 = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("carol"), v("livesIn"), v("porto"), Valid::between(7, 9))?;
            Ok(())
        })
        .unwrap();
    let c = db.now().conflicts(&ConflictQuery::default()).unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].overlaps, vec![Valid::between(7, 9)]);
    assert_eq!(c[0].values.len(), 2);
    let lisbon = &c[0].values[0];
    assert_eq!(lisbon.o, id(&db, &v("lisbon")));
    let eids: Vec<_> = lisbon.statements.iter().map(|s| s.eid).collect();
    assert_eq!(eids, vec![r.asserted[0], r.asserted[1]]);
    let layer: Vec<_> = lisbon
        .statements
        .iter()
        .map(|s| db.now().decode(s.source_layer[0]).unwrap())
        .collect();
    assert_eq!(layer, vec![Value::str("email"), Value::str("chat")]);
    assert_eq!(c[0].values[1].statements[0].eid, r2.asserted[0]);
}

// @lat: [[tests#Memory Conflict Review#Conflict Evidence Is Attributed]]
#[test]
fn every_value_carries_attributed_evidence_and_no_invented_score() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    let first = db
        .transact(TxOptions::default(), |tx| {
            tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent7"))?;
            tx.meta(Value::iri(vocab::SYS_SOURCE), v("crm"))?;
            let e = tx
                .assert(v("dan"), v("age"), Value::Int(41), Valid::ALWAYS)?
                .eid();
            tx.assert(e, v("confidence"), Value::Double(0.9), Valid::ALWAYS)?;
            tx.assert(e, v("confidence"), Value::Int(1), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    let second = db
        .transact(TxOptions::default(), |tx| {
            tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent9"))?;
            tx.assert(v("dan"), v("age"), Value::Int(42), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    let confirm = db
        .transact(TxOptions::default(), |tx| {
            tx.meta(Value::iri(vocab::SYS_AUTHOR), v("agent3"))?;
            tx.confirm(first.asserted[2])?;
            Ok(())
        })
        .unwrap();
    // two confidence values on one statement are a conflict of their own
    let all = db.now().conflicts(&ConflictQuery::default()).unwrap();
    assert_eq!(all.len(), 2);
    let dan = ConflictQuery {
        subject: Some(id(&db, &v("dan"))),
        ..ConflictQuery::default()
    };
    let c = db.now().conflicts(&dan).unwrap();
    assert_eq!(c.len(), 1);
    let [old, new] = [&c[0].values[0], &c[0].values[1]];
    assert_eq!(db.now().decode(old.o).unwrap(), Value::Int(41));
    let e = &old.statements[0];
    assert_eq!(e.eid, first.asserted[2]); // after the two metadata statements
    assert_eq!(e.valid, Valid::ALWAYS);
    assert_eq!(e.t_add, first.t);
    assert_eq!(e.added_at, first.instant);
    assert_eq!(e.confidence, Some(1.0)); // the largest stated value, as stated
    assert_eq!(e.confirmed_by, vec![confirm.t]);
    let names = |ids: &[ObjectId]| -> Vec<Value> {
        ids.iter().map(|i| db.now().decode(*i).unwrap()).collect()
    };
    assert_eq!(names(&e.authors), vec![v("agent7"), v("agent3")]);
    assert_eq!(names(&e.sources), vec![v("crm")]);
    assert!(e.source_layer.is_empty());
    let n = &new.statements[0];
    assert_eq!(n.eid, second.asserted[1]);
    assert_eq!(n.confidence, None); // absent, never estimated
    assert!(n.confirmed_by.is_empty());
    assert_eq!(names(&n.authors), vec![v("agent9")]);
    assert!(n.sources.is_empty());
    // custom layer predicates
    let q = ConflictQuery {
        confidence: Some(id(&db, &v("age"))),
        source: Some(id(&db, &v("confidence"))),
        ..dan
    };
    let c = db.now().conflicts(&q).unwrap();
    let e = &c[0].values[0].statements[0];
    assert_eq!(e.confidence, None);
    assert_eq!(e.source_layer.len(), 2);
}

// @lat: [[tests#Memory Conflict Review#Conflicts Follow The View And Filters]]
#[test]
fn conflicts_follow_the_view_filters_and_declared_cardinality() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(
                v("tags"),
                Value::iri(vocab::SYS_CARDINALITY),
                Value::iri(vocab::SYS_MANY),
                Valid::ALWAYS,
            )?;
            let e = tx
                .assert(v("erin"), v("worksAt"), v("acme"), Valid::ALWAYS)?
                .eid();
            tx.assert(v("erin"), v("worksAt"), v("initech"), Valid::ALWAYS)?;
            tx.assert(v("erin"), v("tags"), Value::str("red"), Valid::ALWAYS)?;
            tx.assert(v("erin"), v("tags"), Value::str("blue"), Valid::ALWAYS)?;
            tx.assert(v("finn"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
            tx.assert(v("finn"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
            // two graph memberships of one statement: sys: predicates are skipped
            tx.add_to_graph(e, v("g1"), AssertOpts::default())?;
            tx.add_to_graph(e, v("g2"), AssertOpts::default())?;
            Ok(())
        })
        .unwrap();
    let now = db.now();
    let all = now.conflicts(&ConflictQuery::default()).unwrap();
    assert_eq!(all.len(), 3, "{all:?}");
    // multi-valued predicates are reported neutrally, never as a schema error
    let tags = all.iter().find(|c| c.p == id(&db, &v("tags"))).unwrap();
    assert!(tags.declared_many);
    assert!(all
        .iter()
        .filter(|c| c.p != tags.p)
        .all(|c| !c.declared_many));
    // filters and the limit
    let erin = ConflictQuery {
        subject: Some(id(&db, &v("erin"))),
        ..ConflictQuery::default()
    };
    assert_eq!(now.conflicts(&erin).unwrap().len(), 2);
    let works = ConflictQuery {
        predicate: Some(id(&db, &v("worksAt"))),
        ..ConflictQuery::default()
    };
    assert_eq!(now.conflicts(&works).unwrap().len(), 2);
    let both = ConflictQuery {
        subject: Some(id(&db, &v("erin"))),
        predicate: Some(id(&db, &v("worksAt"))),
        ..ConflictQuery::default()
    };
    assert_eq!(now.conflicts(&both).unwrap().len(), 1);
    let one = ConflictQuery {
        limit: Some(1),
        ..ConflictQuery::default()
    };
    assert_eq!(now.conflicts(&one).unwrap(), all[..1].to_vec());
    // naming a sys: predicate inspects it
    let member = ConflictQuery {
        predicate: Some(id(&db, &Value::iri(vocab::SYS_IN_GRAPH))),
        ..ConflictQuery::default()
    };
    assert_eq!(now.conflicts(&member).unwrap().len(), 1);
    // resolve finn by superseding; the as-of view still holds the disagreement
    let finn = now
        .triples(Some(id(&db, &v("finn"))), None, Some(id(&db, &v("globex"))))
        .unwrap()[0]
        .eid;
    db.transact(TxOptions::default(), |tx| {
        tx.retract(finn)?;
        Ok(())
    })
    .unwrap();
    let finn_q = ConflictQuery {
        subject: Some(id(&db, &v("finn"))),
        ..ConflictQuery::default()
    };
    assert!(db.now().conflicts(&finn_q).unwrap().is_empty());
    assert_eq!(
        db.as_of(TimeRef::Tx(r.t.0))
            .conflicts(&finn_q)
            .unwrap()
            .len(),
        1
    );
    // history mixes beliefs that never coexisted: refused
    assert!(matches!(
        db.history().conflicts(&ConflictQuery::default()),
        Err(Error::Unsupported { .. })
    ));
    // budgets apply
    let budget = QueryBudget {
        max_rows: Some(1),
        ..QueryBudget::default()
    };
    assert!(matches!(
        db.now()
            .with_budget(&budget)
            .conflicts(&ConflictQuery::default()),
        Err(Error::ResultLimitExceeded { .. })
    ));
}

// @lat: [[tests#Memory Conflict Review#Inspection Never Writes]]
#[test]
fn conflict_inspection_never_retracts_supersedes_or_confirms() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("gus"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("gus"), v("worksAt"), v("initech"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    let before = footprint(&db);
    let counters = db
        .read_sql("SELECT key, value FROM meta ORDER BY key")
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            db.now().conflicts(&ConflictQuery::default()).unwrap().len(),
            1
        );
    }
    assert_eq!(footprint(&db), before);
    assert_eq!(
        db.read_sql("SELECT key, value FROM meta ORDER BY key")
            .unwrap(),
        counters
    );
    // inside a speculation it sees the hypothetical state, and keeps nothing
    let seen = db
        .with(
            |tx| {
                tx.assert(v("gus"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
                Ok(())
            },
            |view| Ok(view.conflicts(&ConflictQuery::default())?[0].values.len()),
        )
        .unwrap();
    assert_eq!(seen, 3);
    assert_eq!(
        db.now().conflicts(&ConflictQuery::default()).unwrap()[0]
            .values
            .len(),
        2
    );
}

/// A bundle from a fresh source: `subject p o`, with a confidence layer.
fn bundle_of(dir: &tempfile::TempDir, name: &str, s: &str, p: &str, o: Value) -> Bundle {
    let src = open(dir, name);
    let r = src
        .transact(TxOptions::default(), |tx| {
            let e = tx.assert(v(s), v(p), o, Valid::ALWAYS)?.eid();
            tx.assert(e, v("confidence"), Value::Double(0.7), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    src.now().bundle(r.asserted[0]).unwrap()
}

fn unique_email(db: &Db) -> TxReport {
    db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("email"),
            Value::iri(vocab::SYS_UNIQUE),
            Value::Bool(true),
            Valid::ALWAYS,
        )?;
        Ok(())
    })
    .unwrap()
}

// @lat: [[tests#Memory Conflict Review#Preview Reports Schema Violations]]
#[test]
fn a_preview_reports_a_schema_violation_and_writes_no_graph_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "target.db");
    unique_email(&db);
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("carol"), v("email"), Value::str("c@x.org"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    let bundle = bundle_of(&dir, "src.db", "bob", "email", Value::str("c@x.org"));
    let before = footprint(&db);
    let p = db.preview_bundle(&bundle).unwrap();
    assert!(!p.would_commit());
    assert!(matches!(p.failure, Some(Error::UniqueViolation { .. })));
    assert!(p.import.is_none() && p.report.is_none());
    assert_eq!(p.scope.basis, TxId(2));
    assert_eq!(footprint(&db), before);
    // the same bundle through a real write fails the same way, atomically
    let applied = db.transact(TxOptions::default(), |tx| {
        tx.import_bundle(&bundle).map(|_| ())
    });
    assert!(matches!(applied, Err(Error::UniqueViolation { .. })));
    assert_eq!(footprint(&db), before);
}

// @lat: [[tests#Memory Conflict Review#Preview Refuses Malformed Bundles]]
#[test]
fn a_preview_refuses_malformed_and_cyclic_bundles() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    let st = |local, s, o| BundleStatement {
        local,
        s,
        p: v("about"),
        o,
        valid: Valid::ALWAYS,
    };
    let dangling = Bundle {
        root: 0,
        statements: vec![st(0, BTerm::Value(v("a")), BTerm::Stmt(7))],
    };
    assert!(matches!(
        db.preview_bundle(&dangling),
        Err(Error::InvalidTerm { .. })
    ));
    let cycle = Bundle {
        root: 0,
        statements: vec![
            st(0, BTerm::Stmt(1), BTerm::Value(v("x"))),
            st(1, BTerm::Stmt(0), BTerm::Value(v("y"))),
        ],
    };
    assert!(matches!(
        db.preview_bundle(&cycle),
        Err(Error::Unsupported { .. })
    ));
    assert_eq!(footprint(&db), (0, 0, 0));
}

// @lat: [[tests#Memory Conflict Review#Successful Preview Changes Nothing]]
#[test]
fn a_successful_preview_shows_proposed_and_reused_facts_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "target.db");
    let held = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(
                v("worksAt"),
                Value::iri(vocab::SYS_CARDINALITY),
                Value::iri(vocab::SYS_ONE),
                Valid::ALWAYS,
            )?;
            tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
            tx.assert(v("bob"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    // reuse: the target already holds alice's fact; only the layer is new
    let bundle = bundle_of(&dir, "a.db", "alice", "worksAt", v("acme"));
    let before = footprint(&db);
    let p = db.preview_bundle(&bundle).unwrap();
    assert!(p.would_commit(), "{:?}", p.failure);
    let import = p.import.as_ref().unwrap();
    assert_eq!(import.root, held.asserted[1]);
    assert_eq!(
        import.statements.iter().map(|s| s.new).collect::<Vec<_>>(),
        vec![false, true]
    );
    let report = p.report.as_ref().unwrap();
    assert_eq!(report.existing, vec![held.asserted[1]]);
    assert_eq!(report.asserted.len(), 1);
    assert!(report.retracted.is_empty());
    // the burned ids are disclosed, and the proposed eid is one of them
    assert_eq!(p.burned.statements, report.asserted);
    assert_eq!(p.scope.basis, held.t);
    assert_eq!(p.scope.t, TxId(held.t.0 + 1));
    assert_eq!(footprint(&db), before);
    // dependency changes: bob's bundle replaces globex under cardinality one
    let bundle = bundle_of(&dir, "b.db", "bob", "worksAt", v("initech"));
    let p = db.preview_bundle(&bundle).unwrap();
    let report = p.report.as_ref().unwrap();
    assert_eq!(
        report.retracted,
        vec![(held.asserted[2], RetKind::Cardinality)]
    );
    assert!(p.burned.terms > 0); // initech is new to this dictionary
    assert_eq!(footprint(&db), before);
    // burned ids are never issued: the next real write starts after them
    let next = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("zed"), v("knows"), v("alice"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    assert!(next.asserted[0] > *p.burned.statements.last().unwrap());
    assert_eq!(next.t, TxId(held.t.0 + 1)); // no transaction number was consumed
}

// @lat: [[tests#Memory Conflict Review#Application Revalidates]]
#[test]
fn applying_after_a_change_revalidates_and_commits_or_fails_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "target.db");
    unique_email(&db);
    let bundle = bundle_of(&dir, "src.db", "bob", "email", Value::str("b@x.org"));
    assert!(db.preview_bundle(&bundle).unwrap().would_commit());
    // another writer takes the unique value before the application
    let taken = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(
                v("mallory"),
                v("email"),
                Value::str("b@x.org"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let before = footprint(&db);
    let applied = db.transact(TxOptions::default(), |tx| {
        tx.import_bundle(&bundle).map(|_| ())
    });
    assert!(matches!(applied, Err(Error::UniqueViolation { .. })));
    assert_eq!(footprint(&db), before); // atomic: not even the layer
    assert!(!db.preview_bundle(&bundle).unwrap().would_commit());
    // the state changes back; the application commits the whole bundle at once
    db.transact(TxOptions::default(), |tx| {
        tx.retract(taken.asserted[0])?;
        Ok(())
    })
    .unwrap();
    let applied = db
        .transact(TxOptions::default(), |tx| {
            tx.import_bundle(&bundle).map(|_| ())
        })
        .unwrap();
    assert_eq!(applied.asserted.len(), 2);
    let events = db.events_since(applied.t.0 - 1).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.t == applied.t));
}
