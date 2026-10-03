//! Bulk import sessions with deferred statistics (OpenSpec change `add-bulk-import`):
//! chunked atomic commits, the write lease, one final analysis, progress, and
//! cancellation without losing committed history.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn tmp() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("import.db");
    (d, p)
}

/// Every commit would trigger the statistics upkeep outside a session.
fn eager() -> OpenOptions {
    OpenOptions {
        optimize_every: 1,
        ..OpenOptions::default()
    }
}

fn count(db: &Db, sql: &str) -> i64 {
    match db.read_sql(sql).unwrap()[0][0] {
        SqlValue::Integer(n) => n,
        ref other => panic!("not a count: {other:?}"),
    }
}

/// Rows of the trace a transaction leaves: tx rows, statements, terms.
fn trace(db: &Db) -> (i64, i64, i64) {
    (
        count(db, "SELECT count(*) FROM tx"),
        count(db, "SELECT count(*) FROM triple"),
        count(db, "SELECT count(*) FROM term"),
    )
}

/// STAT4 samples of the statement table: written only by a full `ANALYZE`.
fn stat4_rows(db: &Db) -> i64 {
    db.read_sql("SELECT count(*) FROM sqlite_stat4 WHERE tbl = 'triple'")
        .map(|r| match r[0][0] {
            SqlValue::Integer(n) => n,
            _ => 0,
        })
        .unwrap_or(0)
}

fn chunk_of(tx: &mut Tx<'_>, chunk: usize, n: i64) -> Result<()> {
    for i in 0..n {
        tx.assert(
            v(&format!("c{chunk}-{i}")),
            v("p"),
            Value::Int(i),
            Valid::ALWAYS,
        )?;
    }
    Ok(())
}

// @lat: [[tests#Bulk Import#Later Chunk Fails]]
#[test]
fn later_chunk_fails_alone() {
    let (_d, p) = tmp();
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("email"),
            Value::iri(vocab::SYS_UNIQUE),
            Value::Bool(true),
            Valid::ALWAYS,
        )
        .map(|_| ())
    })
    .unwrap();
    let mut import = db.bulk_import().unwrap();
    let r1 = import
        .chunk(|tx| {
            tx.assert(v("alice"), v("email"), Value::str("a@x"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    let r2 = import
        .chunk(|tx| {
            tx.assert(v("bob"), v("email"), Value::str("b@x"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    let before = trace(&db);
    let failed = import.chunk(|tx| {
        tx.assert(
            v("carol"),
            v("name"),
            Value::str("a term only this chunk interns"),
            Valid::ALWAYS,
        )?;
        tx.assert(v("carol"), v("email"), Value::str("a@x"), Valid::ALWAYS)?;
        Ok(())
    });
    assert!(matches!(failed, Err(Error::UniqueViolation { .. })));
    // the third chunk left no tx row, statement or term
    assert_eq!(trace(&db), before);
    assert_eq!(
        db.now()
            .encode(&Value::str("a term only this chunk interns"))
            .unwrap(),
        None
    );
    let progress = import.progress().clone();
    assert_eq!(progress.chunks, 2);
    assert_eq!(progress.rejected, 1);
    assert_eq!(progress.asserted, 2);
    assert_eq!(progress.txs, vec![r1.t, r2.t]);
    // the session stays usable after a rejected chunk
    let r3 = import
        .chunk(|tx| {
            tx.assert(v("carol"), v("email"), Value::str("c@x"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(r3.t.0, r2.t.0 + 1, "gap-free numbering");
    let summary = import.finish();
    assert_eq!(summary.progress.txs, vec![r1.t, r2.t, r3.t]);
    let email = db.now().encode(&v("email")).unwrap().unwrap();
    assert_eq!(db.now().triples(None, Some(email), None).unwrap().len(), 3);
}

// @lat: [[tests#Bulk Import#Retry An Assertion Chunk]]
#[test]
fn retried_assertion_chunk_reuses_facts() {
    let (_d, p) = tmp();
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    let mut import = db.bulk_import().unwrap();
    let first = import.chunk(|tx| chunk_of(tx, 0, 5)).unwrap();
    let retry = import.chunk(|tx| chunk_of(tx, 0, 5)).unwrap();
    assert_eq!(first.asserted.len(), 5);
    assert!(retry.asserted.is_empty());
    assert_eq!(retry.existing, first.asserted, "the same statements reused");
    let progress = import.cancel();
    assert_eq!(
        (progress.chunks, progress.asserted, progress.existing),
        (2, 5, 5)
    );
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 5);
}

// @lat: [[tests#Bulk Import#Finalize Many Chunks]]
#[test]
fn finalize_runs_one_analysis_after_twenty_chunks() {
    let (_d, p) = tmp();
    let db = Db::open(&p, eager()).unwrap();
    let mut import = db.bulk_import().unwrap();
    for c in 0..20 {
        import.chunk(|tx| chunk_of(tx, c, 50)).unwrap();
    }
    // with `optimize_every: 1` every one of these commits would have analysed
    assert_eq!(db.optimize_runs().unwrap(), 0);
    assert_eq!(stat4_rows(&db), 0, "no chunk ran a full ANALYZE");
    assert!(db.statistics_due().unwrap());
    let cookie = count(&db, "PRAGMA schema_version");
    let summary = import.finish();
    assert!(summary.analyzed);
    assert!(summary.maintenance_error.is_none());
    assert!(!summary.statistics_due);
    assert_eq!(summary.progress.chunks, 20);
    assert_eq!(summary.progress.asserted, 1000);
    assert_eq!(summary.progress.txs.len(), 20);
    assert!(summary.progress.maintenance > Duration::ZERO);
    assert!(stat4_rows(&db) > 0, "finalization ran a full ANALYZE");
    assert!(
        count(&db, "PRAGMA schema_version") > cookie,
        "pooled readers reload the statistics"
    );
    // the per-commit trigger is back to its default
    assert_eq!(db.optimize_runs().unwrap(), 0);
    db.transact(TxOptions::default(), |tx| chunk_of(tx, 99, 1))
        .unwrap();
    assert_eq!(db.optimize_runs().unwrap(), 1);
}

// @lat: [[tests#Bulk Import#Analysis Failure]]
#[test]
fn failed_final_analysis_keeps_committed_chunks() {
    let (_d, p) = tmp();
    let opts = OpenOptions {
        busy_timeout: Duration::from_millis(50),
        ..OpenOptions::default()
    };
    let db = Db::open(&p, opts).unwrap();
    let mut import = db.bulk_import().unwrap();
    let r1 = import.chunk(|tx| chunk_of(tx, 0, 10)).unwrap();
    let r2 = import.chunk(|tx| chunk_of(tx, 1, 10)).unwrap();
    // another process holds the write lock: ANALYZE gets SQLITE_BUSY
    let other = rusqlite::Connection::open(&p).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let summary = import.finish();
    assert!(!summary.analyzed);
    assert!(matches!(summary.maintenance_error, Some(Error::Sqlite(_))));
    assert!(summary.statistics_due);
    assert_eq!(summary.progress.txs, vec![r1.t, r2.t]);
    assert_eq!(summary.progress.asserted, 20);
    other.execute_batch("ROLLBACK").unwrap();
    // the data was never rolled back, and the lease is free again
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 20);
    assert!(!db.import_active());
    assert!(db.statistics_due().unwrap());
    db.optimize().unwrap();
    assert!(!db.statistics_due().unwrap());
}

// @lat: [[tests#Bulk Import#Cancel Between Chunks]]
#[test]
fn cancel_between_chunks_keeps_history_and_frees_writes() {
    let (_d, p) = tmp();
    let db = Db::open(&p, eager()).unwrap();
    let mut import = db.bulk_import().unwrap();
    let r = import.chunk(|tx| chunk_of(tx, 0, 10)).unwrap();
    let progress = import.cancel();
    assert_eq!(progress.txs, vec![r.t]);
    assert_eq!(
        progress.maintenance,
        Duration::ZERO,
        "no analysis on cancel"
    );
    assert!(!db.import_active());
    assert!(db.statistics_due().unwrap());
    assert_eq!(stat4_rows(&db), 0);
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 10);
    // ordinary writes resume, and the first one runs the due upkeep
    db.transact(TxOptions::default(), |tx| chunk_of(tx, 1, 1))
        .unwrap();
    assert!(!db.statistics_due().unwrap());
    assert_eq!(db.optimize_runs().unwrap(), 1);
    assert!(stat4_rows(&db) > 0);

    // dropping a session does the same, and the history survives a reopen
    let mut import = db.bulk_import().unwrap();
    import.chunk(|tx| chunk_of(tx, 2, 10)).unwrap();
    drop(import);
    assert!(!db.import_active());
    assert!(db.statistics_due().unwrap());
    db.now().sparql("INSERT DATA { v:x v:p 1 }").unwrap();
    drop(db);
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 22);
    assert_eq!(db.history().triples(None, None, None).unwrap().len(), 22);
}

// @lat: [[tests#Bulk Import#Read While Importing]]
#[test]
fn reader_sees_last_committed_chunk() {
    let (_d, p) = tmp();
    let db = Arc::new(Db::open(&p, OpenOptions::default()).unwrap());
    let mut import = db.bulk_import_shared().unwrap();
    import.chunk(|tx| chunk_of(tx, 0, 3)).unwrap();
    import
        .chunk(|tx| {
            chunk_of(tx, 1, 4)?;
            // a pooled reader on this thread and a reader on another thread
            assert_eq!(db.now().triples(None, None, None)?.len(), 3);
            let other = db.clone();
            let seen = thread::spawn(move || other.now().triples(None, None, None).unwrap().len())
                .join()
                .unwrap();
            assert_eq!(seen, 3);
            Ok(())
        })
        .unwrap();
    // reads between chunks see the committed chunk, through any front end
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 7);
    let solutions = db.now().sparql("SELECT ?s WHERE { ?s v:p ?o }").unwrap();
    let SparqlResult::Solutions(s) = solutions else {
        panic!("a SELECT result");
    };
    assert_eq!(s.rows.len(), 7);
    import.finish();
}

// @lat: [[tests#Bulk Import#Exclusive Write Lease]]
#[test]
fn session_holds_the_write_lease() {
    let (_d, p) = tmp();
    let db = Arc::new(Db::open(&p, OpenOptions::default()).unwrap());
    let import = db.bulk_import_shared().unwrap();
    assert!(db.import_active());
    assert!(matches!(db.bulk_import(), Err(Error::ImportInProgress)));
    let write = db.transact(TxOptions::default(), |tx| chunk_of(tx, 0, 1));
    assert!(matches!(write, Err(Error::ImportInProgress)));
    let update = db.now().sparql("INSERT DATA { v:a v:p 1 }");
    assert!(matches!(update, Err(Error::ImportInProgress)));
    let cypher = db.cypher_write(TxOptions::default(), "CREATE (:A)", &CypherParams::new());
    assert!(matches!(cypher, Err(Error::ImportInProgress)));
    // a write from another thread is refused too, and nothing was committed
    let other = db.clone();
    let r = thread::spawn(move || other.transact(TxOptions::default(), |_| Ok(())).is_err())
        .join()
        .unwrap();
    assert!(r);
    assert_eq!(count(&db, "SELECT count(*) FROM tx"), 0);
    // the session moves between threads with its shared handle
    let import = thread::spawn(move || {
        let mut import = import;
        import.chunk(|tx| chunk_of(tx, 1, 2)).unwrap();
        import
    })
    .join()
    .unwrap();
    import.finish();
    assert!(!db.import_active());
    db.transact(TxOptions::default(), |tx| chunk_of(tx, 2, 1))
        .unwrap();
    // a session cannot start inside a transaction
    let nested = db.transact(TxOptions::default(), |_| {
        assert!(matches!(db.bulk_import(), Err(Error::Reentrant)));
        Ok(())
    });
    assert!(nested.is_ok());
}

// @lat: [[tests#Bulk Import#Budgeted Chunk]]
#[test]
fn budgeted_chunk_is_rejected_without_trace() {
    let (_d, p) = tmp();
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    let mut import = db.bulk_import().unwrap();
    import.chunk(|tx| chunk_of(tx, 0, 2)).unwrap();
    let before = trace(&db);
    let token = CancelToken::new();
    token.cancel();
    let budget = QueryBudget {
        cancel: Some(token),
        ..Default::default()
    };
    let r = import.chunk_with(TxOptions::default(), Some(&budget), |tx| chunk_of(tx, 1, 2));
    assert!(matches!(r, Err(Error::Cancelled)));
    assert_eq!(trace(&db), before);
    let dry = TxOptions {
        dry_run: true,
        ..TxOptions::default()
    };
    import
        .chunk_with(dry, None, |tx| chunk_of(tx, 2, 2))
        .unwrap();
    assert_eq!(trace(&db).1, before.1);
    let summary = import.finish();
    assert_eq!(
        (summary.progress.chunks, summary.progress.rejected),
        (1, 1),
        "a dry run is neither committed nor rejected"
    );
}
