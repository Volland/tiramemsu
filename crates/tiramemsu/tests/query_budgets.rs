//! Query budgets (OpenSpec change `add-query-budgets`): bounded reader acquisition,
//! deadlines and cancellation of SQL and native execution, atomic writes, and
//! result budgets that cover a whole operation.
#![cfg(all(feature = "sparql", feature = "cypher"))]

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn tmp() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("budget.db");
    (d, p)
}

fn open_with(path: &std::path::Path, opts: OpenOptions) -> Arc<Db> {
    Arc::new(Db::open(path, opts).unwrap())
}

fn one_reader() -> OpenOptions {
    OpenOptions {
        readers: 1,
        ..OpenOptions::default()
    }
}

/// `n` statements `v:n<i> v:p i`, enough for a cross join that never finishes.
fn seed(db: &Db, n: i64) {
    db.transact(TxOptions::default(), |tx| {
        for i in 0..n {
            tx.assert(v(&format!("n{i}")), v("p"), Value::Int(i), Valid::ALWAYS)?;
        }
        Ok(())
    })
    .unwrap();
}

/// A three-way cross join over `v:p`: billions of combinations.
const CROSS: &str = "SELECT (COUNT(*) AS ?c) WHERE { ?a v:p ?x . ?b v:p ?y . ?c2 v:p ?z }";

/// Blocks the only reader: a SPARQL query whose hook waits until released.
/// Returns the release sender and the worker handle once the reader is held.
fn hold_reader(db: &Arc<Db>) -> (Sender<()>, thread::JoinHandle<()>) {
    let (entered_tx, entered_rx) = channel::<()>();
    let (release_tx, release_rx) = channel::<()>();
    let entered = Mutex::new(entered_tx);
    let release: Mutex<Receiver<()>> = Mutex::new(release_rx);
    db.set_query_hook(Some(Arc::new(move || {
        let _ = entered.lock().unwrap().send(());
        let _ = release.lock().unwrap().recv();
    })));
    let worker_db = db.clone();
    let worker = thread::spawn(move || {
        worker_db
            .now()
            .sparql("SELECT ?o WHERE { ?s ?p ?o }")
            .unwrap();
    });
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the holder reached its query");
    (release_tx, worker)
}

/// Runs `f` on a thread and fails the test if it does not finish in time: a lost
/// reader or a left-open snapshot would block it forever.
fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(10))
        .expect("the read finished: the reader was returned")
}

// @lat: [[tests#Query Budgets#Pool Exhaustion Times Out]]
#[test]
fn pool_exhaustion_times_out_and_a_later_read_succeeds() {
    let (_d, p) = tmp();
    let db = open_with(&p, one_reader());
    seed(&db, 3);
    let (release, worker) = hold_reader(&db);

    // a per-operation reader timeout
    let budget = QueryBudget {
        reader_timeout: Some(Duration::from_millis(50)),
        ..Default::default()
    };
    let start = Instant::now();
    let r = db.now().with_budget(&budget).triples(None, None, None);
    assert!(
        matches!(r, Err(Error::PoolTimeout { timeout }) if timeout == Duration::from_millis(50)),
        "{r:?}"
    );
    assert!(start.elapsed() >= Duration::from_millis(50));
    assert_eq!(
        r.unwrap_err().to_string(),
        "no reader became available within 50 ms"
    );

    // a deadline shorter than the reader timeout ends the wait as a deadline
    let budget = QueryBudget {
        timeout: Some(Duration::from_millis(30)),
        reader_timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };
    let r = db.now().with_budget(&budget).triples(None, None, None);
    assert!(matches!(r, Err(Error::DeadlineExceeded { .. })), "{r:?}");

    // the waiting caller can be cancelled too
    let token = CancelToken::new();
    let budget = QueryBudget {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        token.cancel();
    });
    let r = db.now().with_budget(&budget).triples(None, None, None);
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    canceller.join().unwrap();

    // releasing the holder gives the reader back: no capacity was lost
    release.send(()).unwrap();
    worker.join().unwrap();
    db.set_query_hook(None);
    for _ in 0..3 {
        let db = db.clone();
        assert_eq!(
            within(move || db.now().triples(None, None, None).unwrap().len()),
            3
        );
    }
}

// @lat: [[tests#Query Budgets#Default Reader Timeout]]
#[test]
fn open_options_reader_timeout_bounds_every_read() {
    let (_d, p) = tmp();
    let db = open_with(
        &p,
        OpenOptions {
            reader_timeout: Some(Duration::from_millis(40)),
            ..one_reader()
        },
    );
    assert_eq!(OpenOptions::default().reader_timeout, None);
    seed(&db, 2);
    let (release, worker) = hold_reader(&db);
    let r = db.now().triples(None, None, None);
    assert!(matches!(r, Err(Error::PoolTimeout { .. })), "{r:?}");
    // a budget overrides the database default
    let budget = QueryBudget {
        reader_timeout: Some(Duration::from_millis(10)),
        ..Default::default()
    };
    let start = Instant::now();
    let r = db.now().with_budget(&budget).triples(None, None, None);
    assert!(
        matches!(r, Err(Error::PoolTimeout { timeout }) if timeout == Duration::from_millis(10))
    );
    assert!(start.elapsed() < Duration::from_millis(1_000));
    release.send(()).unwrap();
    worker.join().unwrap();
    db.set_query_hook(None);
    assert_eq!(db.now().triples(None, None, None).unwrap().len(), 2);

    // without a pool the writer serves reads, and its wait is bounded the same way
    let (_d2, p2) = tmp();
    let db = open_with(
        &p2,
        OpenOptions {
            readers: 0,
            ..OpenOptions::default()
        },
    );
    seed(&db, 1);
    let (entered_tx, entered_rx) = channel::<()>();
    let (release_tx, release_rx) = channel::<()>();
    let holder_db = db.clone();
    let holder = thread::spawn(move || {
        holder_db
            .transact(TxOptions::default(), |_tx| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
    });
    entered_rx.recv().unwrap();
    let budget = QueryBudget {
        reader_timeout: Some(Duration::from_millis(30)),
        ..Default::default()
    };
    let r = db.now().with_budget(&budget).triples(None, None, None);
    assert!(matches!(r, Err(Error::PoolTimeout { .. })), "{r:?}");
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(
        db.now()
            .with_budget(&budget)
            .triples(None, None, None)
            .unwrap()
            .len(),
        1
    );
}

// @lat: [[tests#Query Budgets#Reader Freed Before The Deadline]]
#[test]
fn a_reader_freed_before_the_deadline_runs_in_one_snapshot() {
    let (_d, p) = tmp();
    let db = open_with(&p, one_reader());
    seed(&db, 1);
    let (release, worker) = hold_reader(&db);
    let waiter_db = db.clone();
    let waiter = thread::spawn(move || {
        let budget = QueryBudget {
            timeout: Some(Duration::from_secs(10)),
            reader_timeout: Some(Duration::from_secs(10)),
            ..Default::default()
        };
        waiter_db
            .now()
            .with_budget(&budget)
            .triples(None, None, None)
    });
    // a write commits while the caller waits; writes never need a reader
    db.transact(TxOptions::default(), |tx| {
        tx.assert(v("x"), v("q"), v("y"), Valid::ALWAYS)?;
        tx.assert(v("y"), v("q"), v("z"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    thread::sleep(Duration::from_millis(50));
    release.send(()).unwrap();
    worker.join().unwrap();
    let rows = waiter.join().unwrap().expect("the reader arrived in time");
    // the read saw one committed state: the seed and the whole second transaction
    assert_eq!(rows.len(), 3);
    db.set_query_hook(None);
}

// @lat: [[tests#Query Budgets#Cancel A Path]]
#[test]
fn cancelling_a_native_path_stops_frontier_expansion_and_frees_the_reader() {
    let (_d, p) = tmp();
    let db = open_with(
        &p,
        OpenOptions {
            path_max_states: usize::MAX,
            ..one_reader()
        },
    );
    // a complete directed graph on 9 nodes: its trails never run out in practice
    db.transact(TxOptions::default(), |tx| {
        for a in 0..9 {
            for b in 0..9 {
                if a != b {
                    tx.assert(
                        v(&format!("k{a}")),
                        v("knows"),
                        v(&format!("k{b}")),
                        Valid::ALWAYS,
                    )?;
                }
            }
        }
        tx.assert(v("k0"), v("id"), Value::str("k0"), Valid::ALWAYS)?;
        Ok(())
    })
    .unwrap();
    let start_node = db.now().encode(&v("k0")).unwrap().unwrap();

    let token = CancelToken::new();
    let budget = QueryBudget {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        token.cancel();
    });
    let t0 = Instant::now();
    let r = db
        .now()
        .with_budget(&budget)
        .path(start_node, "knows*", PathMode::Trail, u32::MAX);
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
    canceller.join().unwrap();

    // a deadline stops the same search, through `tm_path` inside SQL as well
    let budget = QueryBudget {
        timeout: Some(Duration::from_millis(50)),
        ..Default::default()
    };
    let r = db
        .now()
        .with_budget(&budget)
        .path(start_node, "knows*", PathMode::Trail, u32::MAX);
    assert!(matches!(r, Err(Error::DeadlineExceeded { .. })), "{r:?}");
    let r = db.now().with_budget(&budget).cypher(
        "MATCH (x {id: 'k0'})-[:knows*]->(y) RETURN count(*) AS n",
        &CypherParams::default(),
    );
    assert!(matches!(r, Err(Error::DeadlineExceeded { .. })), "{r:?}");

    // the only reader is reusable at once
    let db2 = db.clone();
    let rows = within(move || {
        db2.now()
            .path(start_node, "knows", PathMode::Reachability, 1)
            .unwrap()
    });
    assert_eq!(rows.len(), 8);
}

/// The bundled host with SQL interruption hidden: its executors keep the default
/// `set_interrupt`, which ignores the request, so only native checks can stop work.
struct NoInterruptHost(RusqliteHost);

struct NoInterrupt(Box<dyn Executor>);

impl Executor for NoInterrupt {
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize> {
        self.0.execute(sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()> {
        self.0.query(sql, params, row)
    }
    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.0.execute_batch(sql)
    }
    fn begin_immediate(&mut self) -> Result<()> {
        self.0.begin_immediate()
    }
    fn begin_read(&mut self) -> Result<()> {
        self.0.begin_read()
    }
    fn commit(&mut self) -> Result<()> {
        self.0.commit()
    }
    fn rollback(&mut self) -> Result<()> {
        self.0.rollback()
    }
    fn savepoint(&mut self, name: &str) -> Result<()> {
        self.0.savepoint(name)
    }
    fn rollback_to(&mut self, name: &str) -> Result<()> {
        self.0.rollback_to(name)
    }
    fn release(&mut self, name: &str) -> Result<()> {
        self.0.release(name)
    }
    fn registry(&mut self) -> Option<&mut dyn tm_core::HostRegistry> {
        self.0.registry()
    }
}

impl Host for NoInterruptHost {
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    fn open_writer(&self, path: &std::path::Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(NoInterrupt(self.0.open_writer(path, opts)?)))
    }
    fn open_reader(&self, path: &std::path::Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(NoInterrupt(self.0.open_reader(path, opts)?)))
    }
}

// @lat: [[tests#Query Budgets#Native Checks Without Host Interrupts]]
#[test]
fn native_path_checks_stop_a_search_on_a_host_that_cannot_interrupt() {
    let (_d, p) = tmp();
    let db = Arc::new(
        Db::open_with_host(
            NoInterruptHost(RusqliteHost::new()),
            &p,
            OpenOptions {
                path_max_states: usize::MAX,
                ..one_reader()
            },
        )
        .unwrap(),
    );
    db.transact(TxOptions::default(), |tx| {
        for a in 0..9 {
            for b in 0..9 {
                if a != b {
                    tx.assert(
                        v(&format!("k{a}")),
                        v("knows"),
                        v(&format!("k{b}")),
                        Valid::ALWAYS,
                    )?;
                }
            }
        }
        Ok(())
    })
    .unwrap();
    let k0 = db.now().encode(&v("k0")).unwrap().unwrap();
    let token = CancelToken::new();
    let budget = QueryBudget {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        token.cancel();
    });
    let t0 = Instant::now();
    let r = db
        .now()
        .with_budget(&budget)
        .path(k0, "knows*", PathMode::Trail, u32::MAX);
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
    canceller.join().unwrap();
    let db2 = db.clone();
    assert_eq!(
        within(move || db2
            .now()
            .path(k0, "knows", PathMode::Reachability, 1)
            .unwrap()
            .len()),
        8
    );
}

// @lat: [[tests#Query Budgets#Cancel SQL Execution]]
#[test]
fn deadlines_and_cancellation_interrupt_sql_and_release_the_reader() {
    let (_d, p) = tmp();
    let db = open_with(&p, one_reader());
    seed(&db, 2_000);

    let budget = QueryBudget {
        timeout: Some(Duration::from_millis(100)),
        ..Default::default()
    };
    let t0 = Instant::now();
    let r = db.now().with_budget(&budget).sparql(CROSS);
    assert!(
        matches!(r, Err(Error::DeadlineExceeded { timeout }) if timeout == Duration::from_millis(100)),
        "{r:?}"
    );
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());

    let token = CancelToken::new();
    let budget = QueryBudget {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        token.cancel();
    });
    let r = db.now().with_budget(&budget).sparql(CROSS);
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    canceller.join().unwrap();

    let r = db
        .now()
        .with_budget(&QueryBudget {
            timeout: Some(Duration::from_millis(100)),
            ..Default::default()
        })
        .cypher(
            "MATCH (a), (b), (c) WHERE a.p >= 0 AND b.p >= 0 AND c.p >= 0 RETURN count(*) AS n",
            &CypherParams::default(),
        );
    assert!(matches!(r, Err(Error::DeadlineExceeded { .. })), "{r:?}");

    // the reader is reusable and carries no stale interrupt: once the deadline of
    // the last budget has long passed, a plain query that runs many SQLite steps
    // still succeeds on the same reader
    thread::sleep(Duration::from_millis(150));
    let db2 = db.clone();
    let n = within(move || {
        db2.now()
            .sparql("SELECT (COUNT(*) AS ?c) WHERE { ?a v:p ?x . ?b v:p ?y . FILTER(?x < 50 && ?y < 50) }")
            .unwrap()
            .solutions()
            .unwrap()
            .rows[0][0]
            .clone()
    });
    assert_eq!(n, Some(Value::Int(2_500)));
}

fn tx_count(db: &Db) -> i64 {
    db.read_sql("SELECT count(*) FROM tx").unwrap()[0][0]
        .as_i64()
        .unwrap()
}

fn triple_count(db: &Db) -> i64 {
    db.read_sql("SELECT count(*) FROM triple").unwrap()[0][0]
        .as_i64()
        .unwrap()
}

// @lat: [[tests#Query Budgets#Cancel A Write]]
#[test]
fn an_interrupted_write_commits_nothing() {
    let (_d, p) = tmp();
    let db = open_with(&p, OpenOptions::default());
    seed(&db, 2_000);
    let (t0, s0) = (tx_count(&db), triple_count(&db));
    let events0 = db.events_since(0).unwrap().len();

    // a SPARQL update whose WHERE never finishes
    let budget = QueryBudget {
        timeout: Some(Duration::from_millis(100)),
        ..Default::default()
    };
    let r = db
        .now()
        .with_budget(&budget)
        .sparql("INSERT { ?a v:q ?b } WHERE { ?a v:p ?x . ?b v:p ?y . ?c v:p ?z }");
    assert!(matches!(r, Err(Error::DeadlineExceeded { .. })), "{r:?}");
    assert_eq!((tx_count(&db), triple_count(&db)), (t0, s0));

    // a Cypher write stopped by its token
    let token = CancelToken::new();
    let budget = QueryBudget {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        token.cancel();
    });
    let r = db.cypher_write_budgeted(
        TxOptions::default(),
        "MATCH (a), (b), (c) WHERE a.p >= 0 AND b.p >= 0 AND c.p >= 0 CREATE (a)-[:q]->(b)",
        &CypherParams::default(),
        &budget,
    );
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    canceller.join().unwrap();
    assert_eq!((tx_count(&db), triple_count(&db)), (t0, s0));

    // a transaction body that wrote and then ran past its deadline: the check when
    // the body returns rolls it back
    let budget = QueryBudget {
        timeout: Some(Duration::from_millis(20)),
        ..Default::default()
    };
    let r = db.transact_budgeted(TxOptions::default(), &budget, |tx| {
        tx.assert(v("late"), v("q"), v("write"), Valid::ALWAYS)?;
        thread::sleep(Duration::from_millis(40));
        Ok(())
    });
    assert!(matches!(r, Err(Error::DeadlineExceeded { .. })), "{r:?}");
    assert_eq!((tx_count(&db), triple_count(&db)), (t0, s0));
    assert_eq!(db.events_since(0).unwrap().len(), events0);
    assert_eq!(db.now().encode(&v("late")).unwrap(), None);

    // the writer carries no stale interrupt: long past the deadline, a large
    // write commits with the next gap-free number
    thread::sleep(Duration::from_millis(50));
    let report = db
        .transact_budgeted(
            TxOptions::default(),
            &QueryBudget {
                timeout: Some(Duration::from_secs(30)),
                ..Default::default()
            },
            |tx| {
                for i in 0..500 {
                    tx.assert(v(&format!("m{i}")), v("q"), Value::Int(i), Valid::ALWAYS)?;
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(report.t, TxId(t0 as u64 + 1));
    let report = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("after"), v("q"), v("all"), Valid::ALWAYS)
                .map(|_| ())
        })
        .unwrap();
    assert_eq!(report.t, TxId(t0 as u64 + 2));
}

// @lat: [[tests#Query Budgets#Result Overflow Fails]]
#[test]
fn exceeding_a_result_budget_fails_instead_of_truncating() {
    let (_d, p) = tmp();
    let db = open_with(&p, OpenOptions::default());
    seed(&db, 10);
    let q = "SELECT ?s ?o WHERE { ?s v:p ?o }";
    let rows = |b: &QueryBudget| -> Result<usize> {
        Ok(db
            .now()
            .with_budget(b)
            .sparql(q)?
            .solutions()
            .unwrap()
            .rows
            .len())
    };
    let max_rows = |n| QueryBudget {
        max_rows: Some(n),
        ..Default::default()
    };
    assert_eq!(rows(&max_rows(10)).unwrap(), 10);
    let r = rows(&max_rows(9));
    assert!(
        matches!(
            r,
            Err(Error::ResultLimitExceeded {
                limit: ResultLimit::Rows(9)
            })
        ),
        "{r:?}"
    );
    assert_eq!(
        r.unwrap_err().to_string(),
        "result exceeds the limit of 9 rows"
    );
    let max_bytes = |n| QueryBudget {
        max_bytes: Some(n),
        ..Default::default()
    };
    assert_eq!(rows(&max_bytes(1_000_000)).unwrap(), 10);
    let r = rows(&max_bytes(100));
    assert!(
        matches!(
            r,
            Err(Error::ResultLimitExceeded {
                limit: ResultLimit::Bytes(100)
            })
        ),
        "{r:?}"
    );

    // the other read surfaces honour the same budget
    let tight = max_rows(3);
    let view = db.now().with_budget(&tight);
    assert!(matches!(
        view.triples(None, None, None),
        Err(Error::ResultLimitExceeded { .. })
    ));
    assert!(matches!(
        view.cypher(
            "MATCH (s) WHERE s.p >= 0 RETURN s, s.p AS o",
            &CypherParams::default()
        ),
        Err(Error::ResultLimitExceeded { .. })
    ));
    let n0 = db.now().encode(&v("n0")).unwrap().unwrap();
    db.transact(TxOptions::default(), |tx| {
        for i in 0..5 {
            tx.assert(
                v(&format!("c{i}")),
                v("next"),
                v(&format!("c{}", i + 1)),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    })
    .unwrap();
    let c0 = db.now().encode(&v("c0")).unwrap().unwrap();
    assert!(matches!(
        view.path(c0, "v:next+", PathMode::Reachability, u32::MAX),
        Err(Error::ResultLimitExceeded { .. })
    ));
    assert!(matches!(
        view.events_since(0),
        Err(Error::ResultLimitExceeded { .. })
    ));
    // small results pass, and the same view runs each call as its own operation
    assert_eq!(view.triples(Some(n0), None, None).unwrap().len(), 1);
    assert_eq!(view.triples(Some(n0), None, None).unwrap().len(), 1);
    assert_eq!(
        view.path(c0, "v:next", PathMode::Reachability, 1)
            .unwrap()
            .len(),
        1
    );

    // a Cypher write whose rows overflow commits nothing
    let before = triple_count(&db);
    let r = db.cypher_write_budgeted(
        TxOptions::default(),
        "MATCH (s) WHERE s.p >= 0 CREATE (s)-[:seen]->(:Mark) RETURN s",
        &CypherParams::default(),
        &max_rows(5),
    );
    assert!(matches!(r, Err(Error::ResultLimitExceeded { .. })), "{r:?}");
    assert_eq!(triple_count(&db), before);
}

// @lat: [[tests#Query Budgets#Composite Operations Share One Budget]]
#[test]
fn provenance_lookups_draw_on_the_same_operation_budget() {
    let (_d, p) = tmp();
    let db = open_with(&p, OpenOptions::default());
    db.now()
        .sparql("INSERT DATA { v:alice v:worksAt v:acme . v:acme v:in v:paris . v:bob v:worksAt v:acme }")
        .unwrap();
    let q = "SELECT ?who ?c WHERE { ?who v:worksAt ?o . ?o v:in ?c }";
    let plain = db.now().sparql(q).unwrap().solutions().unwrap().rows.len();
    assert_eq!(plain, 2);
    let budget = QueryBudget {
        max_rows: Some(plain as u64),
        ..Default::default()
    };
    // the plain query fits the budget exactly
    assert_eq!(
        db.now()
            .with_budget(&budget)
            .sparql(q)
            .unwrap()
            .solutions()
            .unwrap()
            .rows
            .len(),
        plain
    );
    // with provenance the sibling lookups need rows too, from the same budget
    let opts = SparqlOptions {
        provenance: true,
        ..Default::default()
    };
    let r = db.now().with_budget(&budget).sparql_with(q, &opts);
    assert!(
        matches!(
            r,
            Err(Error::ResultLimitExceeded {
                limit: ResultLimit::Rows(2)
            })
        ),
        "{r:?}"
    );
    // a budget large enough for the whole operation gives the full cited answer
    let roomy = QueryBudget {
        max_rows: Some(1_000),
        ..Default::default()
    };
    let r = db.now().with_budget(&roomy).sparql_with(q, &opts).unwrap();
    assert_eq!(r.solutions().unwrap().provenance(0).unwrap().len(), 2);

    // a Cypher query's clauses run as separate statements that draw on one budget:
    // each returns two rows, which the budget allows once but not twice
    let cy = "MATCH (a)-[:worksAt]->(o) WITH a, o MATCH (o)-[:in]->(c) RETURN a, c";
    assert_eq!(
        db.now()
            .cypher(cy, &CypherParams::default())
            .unwrap()
            .rows
            .len(),
        2
    );
    let r = db
        .now()
        .with_budget(&budget)
        .cypher(cy, &CypherParams::default());
    assert!(matches!(r, Err(Error::ResultLimitExceeded { .. })), "{r:?}");
    let r = db
        .now()
        .with_budget(&roomy)
        .cypher(cy, &CypherParams::default());
    assert_eq!(r.unwrap().rows.len(), 2);
}

// @lat: [[tests#Query Budgets#Default Budget Changes Nothing]]
#[test]
fn an_empty_budget_changes_nothing() {
    let (_d, p) = tmp();
    let db = open_with(&p, OpenOptions::default());
    seed(&db, 20);
    let none = QueryBudget::default();
    let q = "SELECT ?s ?o WHERE { ?s v:p ?o } ORDER BY ?o";
    assert_eq!(
        db.now().with_budget(&none).sparql(q).unwrap(),
        db.now().sparql(q).unwrap()
    );
    assert_eq!(
        db.now()
            .with_budget(&none)
            .triples(None, None, None)
            .unwrap(),
        db.now().triples(None, None, None).unwrap()
    );
    let budgeted = db.now().with_budget(&none);
    assert!(budgeted.budget().is_some());
    assert!(db.now().budget().is_none());
    // a budget keeps the view's time selection
    let past = db.as_of(TimeRef::Tx(0)).with_budget(&none);
    assert!(past.triples(None, None, None).unwrap().is_empty());
    let report = db
        .transact_budgeted(TxOptions::default(), &none, |tx| {
            tx.assert(v("x"), v("q"), v("y"), Valid::ALWAYS).map(|_| ())
        })
        .unwrap();
    assert_eq!(report.t, TxId(2));
    // budgeted reads keep running in parallel on the pool
    let mut workers = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        workers.push(thread::spawn(move || {
            let b = QueryBudget {
                timeout: Some(Duration::from_secs(30)),
                max_rows: Some(1_000),
                ..Default::default()
            };
            db.now()
                .with_budget(&b)
                .sparql("SELECT ?s WHERE { ?s v:p ?o }")
                .unwrap()
        }));
    }
    for w in workers {
        assert_eq!(w.join().unwrap().solutions().unwrap().rows.len(), 20);
    }
}
