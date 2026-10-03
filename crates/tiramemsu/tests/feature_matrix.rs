//! Spec `optional-query-frontends`: runtime smoke tests for every facade feature
//! combination (core, exec, sparql, cypher, default). The tests without a `cfg`
//! run in every build, so the core tier is exercised with and without the query
//! front ends; `scripts/feature-matrix.sh` checks the dependency trees.

use std::time::Duration;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn open(dir: &tempfile::TempDir, name: &str) -> Db {
    Db::open(dir.path().join(name), OpenOptions::default()).unwrap()
}

/// The facts every handoff and core test writes: one fact with a layer, then a
/// correction, so the now, as-of and history views differ.
fn write_story(db: &Db) -> (Eid, Eid) {
    let r = db
        .transact(TxOptions::default(), |tx| {
            let job = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::from(10))?
                .eid();
            tx.assert(job, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
            tx.assert(
                v("alice"),
                v("note"),
                Value::str("met at the Lisbon offsite"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let job = r.asserted[0];
    let mut new = None;
    db.transact(TxOptions::default(), |tx| {
        let mut patch = Patch::object(v("initech"));
        patch.v_from = Some(Some(20));
        new = Some(tx.supersede(job, patch)?);
        Ok(())
    })
    .unwrap();
    (job, new.unwrap())
}

/// What a reader of the story sees through the core API, as comparable text.
fn read_story(db: &Db) -> Vec<String> {
    let render = |view: View<'_>| -> Vec<String> {
        let mut out: Vec<String> = view
            .triples(None, None, None)
            .unwrap()
            .iter()
            .map(|t| {
                format!(
                    "{} {} {} {:?} {:?}",
                    view.decode(t.s).unwrap().lexical(),
                    view.decode(t.p).unwrap().lexical(),
                    view.decode(t.o).unwrap().lexical(),
                    t.valid(),
                    t.t_ret,
                )
            })
            .collect();
        out.sort();
        out
    };
    let mut all = vec!["now".to_string()];
    all.extend(render(db.now()));
    all.push("as-of 1".into());
    all.extend(render(db.as_of(TimeRef::Tx(1))));
    all.push("history".into());
    all.extend(render(db.history()));
    all.push("valid at 15".into());
    all.extend(render(db.as_of(TimeRef::Tx(1)).valid_at(15)));
    all
}

// optional-query-frontends "Core operations": a build asserts and reads a fact with
// the same persisted format and temporal semantics, whatever its features
// @lat: [[tests#Optional Query Frontends#Core Operations In Every Build]]
#[test]
fn core_operations_work_in_every_build() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("m.db");
    {
        let db = Db::open(&p, OpenOptions::default()).unwrap();
        let (job, new) = write_story(&db);
        // now: the corrected fact; as of t1: the original; history: both
        let now = db.now().triples(None, None, None).unwrap();
        assert!(now.iter().any(|t| t.eid == new));
        assert!(!now.iter().any(|t| t.eid == job));
        let then = db.as_of(TimeRef::Tx(1));
        assert!(then
            .triples(None, None, None)
            .unwrap()
            .iter()
            .any(|t| t.eid == job));
        let hist = db.history().triples(None, None, None).unwrap();
        let old = hist.iter().find(|t| t.eid == job).unwrap();
        assert_eq!(old.t_ret, Some(TxId(2)));
        // valid time is half open: the original holds from 10
        assert!(
            then.valid_at(9).triples(None, None, None).unwrap().len()
                < then.triples(None, None, None).unwrap().len()
        );
        assert!(!db.events_since(1).unwrap().is_empty());
        assert!(db.events_since(2).unwrap().is_empty());
    }
    // the file carries the one storage format of this version, and reopening it
    // shows the same story
    let rows = {
        let db = Db::open(&p, OpenOptions::default()).unwrap();
        let version = db
            .read_sql("SELECT value FROM meta WHERE key = 'format_version'")
            .unwrap();
        assert_eq!(
            version[0][0],
            SqlValue::Integer(tm_core::storage::FORMAT_VERSION)
        );
        read_story(&db)
    };
    let again = Db::open(&p, OpenOptions::default()).unwrap();
    assert_eq!(read_story(&again), rows);
}

// optional-query-frontends "Core-only dependency graph": bundle JSON and N-Triples
// need no query front end
// @lat: [[tests#Optional Query Frontends#Bundle Formats Without A Parser]]
#[test]
fn bundle_formats_without_a_parser() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    let r = a
        .transact(TxOptions::default(), |tx| {
            let job = tx
                .assert(v("alice"), v("worksAt"), v("acme"), Valid::between(5, 50))?
                .eid();
            tx.assert(job, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    let bundle = a.now().bundle(r.asserted[0]).unwrap();
    let text = bundle.to_json().to_string();
    let back = Bundle::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(back, bundle);
    assert_eq!(bundle.to_json()["format"], BUNDLE_FORMAT);
    let nt = bundle.to_ntriples();
    assert!(nt.contains("<http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies>"));
    assert!(nt.contains(
        "<<( <urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> <urn:tiramemsu:v:acme> )>>"
    ));
    assert!(nt.contains("validFrom"));
    // the preview and the import are core operations too
    assert!(b.preview_bundle(&back).unwrap().would_commit());
    let applied = b
        .transact(TxOptions::default(), |tx| {
            tx.import_bundle(&back).map(|_| ())
        })
        .unwrap();
    assert_eq!(applied.asserted.len(), 2);
    assert_eq!(b.now().bundle(applied.asserted[0]).unwrap(), bundle);
    // the RDF terms are the same types in every build
    let term: RdfTerm = tm_core::rdf::render(&v("alice"));
    assert_eq!(term, RdfTerm::Iri("urn:tiramemsu:v:alice".into()));
}

// The recent core features stay in the core tier: text recall, conflict review,
// bulk import and operation budgets.
// @lat: [[tests#Optional Query Frontends#Recent Core Features In Every Build]]
#[test]
fn recent_core_features_in_every_build() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(
        dir.path().join("m.db"),
        OpenOptions {
            text_index: true,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    write_story(&db);
    let hits = db.now().text_search(&TextQuery::new("lisbon")).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "met at the Lisbon offsite");
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("bob"), v("worksAt"), v("globex"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    let conflicts = db.now().conflicts(&ConflictQuery::default()).unwrap();
    assert_eq!(conflicts.len(), 1);
    let mut import = db.bulk_import().unwrap();
    import
        .chunk(|tx| {
            for i in 0..10 {
                tx.assert(
                    v(&format!("doc{i}")),
                    v("mentions"),
                    v("acme"),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        db.transact(TxOptions::default(), |_| Ok(())),
        Err(Error::ImportInProgress)
    ));
    let summary = import.finish();
    assert_eq!(summary.progress.asserted, 10);
    let budget = QueryBudget {
        timeout: Some(Duration::from_secs(5)),
        max_rows: Some(3),
        ..Default::default()
    };
    assert!(matches!(
        db.now().with_budget(&budget).triples(None, None, None),
        Err(Error::ResultLimitExceeded { .. })
    ));
    let cancel = CancelToken::new();
    cancel.cancel();
    let stopped = QueryBudget {
        cancel: Some(cancel),
        ..Default::default()
    };
    assert!(matches!(
        db.now().with_budget(&stopped).triples(None, None, None),
        Err(Error::Cancelled)
    ));
}

// A file is shared between builds: `scripts/feature-matrix.sh handoff` writes it
// with the core build and reads it with the default build (and the reverse). Only
// runs when `TIRAMEMSU_HANDOFF` names the file.
// @lat: [[tests#Optional Query Frontends#One File Across Builds]]
#[test]
fn handoff_file_across_builds() {
    let Some(p) = std::env::var_os("TIRAMEMSU_HANDOFF") else {
        return;
    };
    let p = std::path::PathBuf::from(p);
    let expect = std::path::PathBuf::from(format!("{}.story", p.display()));
    if !p.exists() {
        let db = Db::open(&p, OpenOptions::default()).unwrap();
        write_story(&db);
        std::fs::write(&expect, read_story(&db).join("\n")).unwrap();
        return;
    }
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    let story = std::fs::read_to_string(&expect).unwrap();
    assert_eq!(read_story(&db).join("\n"), story);
    #[cfg(feature = "sparql")]
    {
        let r = db
            .now()
            .sparql("SELECT ?c WHERE { v:alice v:worksAt ?c }")
            .unwrap();
        assert_eq!(r.solutions().unwrap().get(0, "c"), Some(&v("initech")));
    }
}

// optional-query-frontends "Single frontend" and the execution-only build: the
// shared engine evaluates paths and IR without either parser
// @lat: [[tests#Optional Query Frontends#Shared Execution Without A Parser]]
#[test]
#[cfg(feature = "exec")]
fn shared_execution_without_a_parser() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(
        dir.path().join("m.db"),
        OpenOptions {
            path_max_hops: 4,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("a"), v("knows"), v("b"), Valid::ALWAYS)?;
        tx.assert(v("b"), v("knows"), v("c"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(db.path_max_hops(), 4);
    let view = db.now();
    let a = view.encode(&v("a")).unwrap().unwrap();
    let rows = view
        .path(a, "knows+", PathMode::Reachability, u32::MAX)
        .unwrap();
    assert_eq!(rows.len(), 2);
    let report = view.path_report(a, "knows+", &PathArgs::default()).unwrap();
    assert_eq!(report.completeness, PathCompleteness::Exhaustive);
    // the tm-core tier is still available on request
    let core = Db::open(
        dir.path().join("core.db"),
        OpenOptions {
            query_engine: false,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    assert!(matches!(
        core.now()
            .path(a, "knows+", PathMode::Reachability, u32::MAX),
        Err(Error::Unsupported { .. })
    ));
}

// optional-query-frontends "Single frontend": SPARQL with shared execution, and
// no Cypher parser linked (the dependency side is checked by the script)
// @lat: [[tests#Optional Query Frontends#SPARQL Only]]
#[test]
#[cfg(feature = "sparql")]
fn sparql_frontend_runs_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    db.now()
        .sparql("INSERT DATA { v:a v:knows v:b . v:b v:knows v:c }")
        .unwrap();
    let r = db
        .now()
        .sparql("SELECT ?x WHERE { v:a v:knows+ ?x } ORDER BY ?x")
        .unwrap();
    assert_eq!(r.solutions().unwrap().rows.len(), 2);
    let explain = db
        .now()
        .explain_sparql("SELECT ?x WHERE { v:a v:knows ?x }");
    assert!(explain.is_ok());
}

// optional-query-frontends "Single frontend" for Cypher
// @lat: [[tests#Optional Query Frontends#Cypher Only]]
#[test]
#[cfg(feature = "cypher")]
fn cypher_frontend_runs_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    db.cypher_write(
        TxOptions::default(),
        "CREATE (:Person {name: 'Ada'})-[:KNOWS]->(:Person {name: 'Bob'})",
        &CypherParams::new(),
    )
    .unwrap();
    let r = db
        .now()
        .cypher(
            "MATCH (a:Person)-[:KNOWS*1..]->(b) RETURN b.name AS name",
            &CypherParams::new(),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    // and inside a transaction, through the extension trait
    db.transact(TxOptions::default(), |tx| {
        TxCypher::cypher(tx, "CREATE (:Person {name: 'Cy'})", &CypherParams::new()).map(|_| ())
    })
    .unwrap();
}

// optional-query-frontends "Default application": the default build keeps both
// front ends, paths and saved answers
// @lat: [[tests#Optional Query Frontends#Default Surface]]
#[test]
#[cfg(all(feature = "sparql", feature = "cypher"))]
fn default_surface_keeps_every_frontend() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, "m.db");
    db.now().sparql("INSERT DATA { v:a v:knows v:b }").unwrap();
    let c = db
        .now()
        .cypher("MATCH (a)-[:knows]->(b) RETURN b", &CypherParams::new())
        .unwrap();
    assert_eq!(c.rows.len(), 1);
    let saved = db
        .save_answer(
            "who",
            &SavedQuery::sparql("SELECT ?x WHERE { v:a v:knows ?x }"),
        )
        .unwrap();
    assert!(saved.is_fresh());
}
