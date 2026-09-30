//! Shared fixtures for the tm-exec integration tests: temporary databases, the
//! alice/acme/globex/belief layer fixture of `lat.md/data-model#Layers`, and the
//! skewed plan fixture of `lat.md/tests#Query#Skewed Joins Start Selective`.
#![allow(dead_code)]

pub mod mock;
pub mod probe;
pub mod skewed;

use std::collections::BTreeMap;
use std::sync::Arc;

use tiramemsu::*;

/// An IRI in the default vocabulary (`v:local`).
pub fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// The IRI text of `v:local`.
pub fn vi(local: &str) -> String {
    format!("urn:tiramemsu:v:{local}")
}

/// A plain string value.
pub fn s(text: &str) -> Value {
    Value::str(text)
}

/// A temporary database with a manual clock starting at `t0` (1 ms per tx).
pub struct TestDb {
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub clock: Arc<ManualClock>,
    pub path: std::path::PathBuf,
}

impl TestDb {
    pub fn open(opts: OpenOptions) -> TestDb {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("q.db");
        let clock = Arc::new(ManualClock::new(1_000));
        let db = Db::open(
            &path,
            OpenOptions {
                clock: clock.clone(),
                ..opts
            },
        )
        .expect("open");
        TestDb {
            dir,
            db,
            clock,
            path,
        }
    }

    pub fn new() -> TestDb {
        TestDb::open(OpenOptions::default())
    }

    /// Runs a transaction that must commit.
    pub fn tx<F: FnOnce(&mut Tx<'_>) -> Result<()>>(&self, f: F) -> TxReport {
        self.db.transact(TxOptions::default(), f).expect("commit")
    }

    /// Commits empty transactions until the last committed `t` is `t`.
    pub fn advance_to(&self, t: u64) {
        loop {
            let last = self.last_t();
            if last >= t {
                assert_eq!(last, t, "already past tx {t}");
                return;
            }
            self.tx(|_| Ok(()));
        }
    }

    /// The last committed transaction number.
    pub fn last_t(&self) -> u64 {
        self.db
            .read_sql("SELECT value FROM meta WHERE key = 'last_t'")
            .unwrap()[0][0]
            .as_i64()
            .unwrap() as u64
    }

    /// The ObjectId of a stored value.
    pub fn id(&self, val: &Value) -> ObjectId {
        self.db.now().encode(val).unwrap().expect("stored value")
    }
}

/// The layer fixture: `e1 = (alice worksAt acme)` annotated by `e2 = (e1 confidence
/// 0.8)`, confirmed by `e3 = (e1 sys:confirmedBy tx2)`, referenced by the belief
/// `e7 = (belief9 supportedBy e1)`, which is annotated by `e8 = (e7 method
/// "llm-extraction")`. Also `acme name "Acme Corp"`, `globex name "Globex"` and
/// `alice name "Alice"`. Eids are exposed by name.
pub struct Layers {
    pub t: TestDb,
    pub eids: BTreeMap<&'static str, Eid>,
}

impl Layers {
    pub fn eid(&self, name: &str) -> Eid {
        self.eids[name]
    }

    pub fn db(&self) -> &Db {
        &self.t.db
    }
}

pub fn layers() -> Layers {
    let t = TestDb::new();
    let mut eids = BTreeMap::new();
    let mut e = BTreeMap::new();
    t.tx(|tx| {
        let e1 = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        let e2 = tx
            .assert(e1, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?
            .eid();
        let e7 = tx
            .assert(v("belief9"), v("supportedBy"), e1, Valid::ALWAYS)?
            .eid();
        let e8 = tx
            .assert(e7, v("method"), s("llm-extraction"), Valid::ALWAYS)?
            .eid();
        tx.assert(v("acme"), v("name"), s("Acme Corp"), Valid::ALWAYS)?;
        tx.assert(v("globex"), v("name"), s("Globex"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("name"), s("Alice"), Valid::ALWAYS)?;
        e.insert("e1", e1);
        e.insert("e2", e2);
        e.insert("e7", e7);
        e.insert("e8", e8);
        Ok(())
    });
    eids.extend(e);
    let e1 = eids["e1"];
    let mut e3 = None;
    t.tx(|tx| {
        e3 = Some(tx.confirm(e1)?);
        Ok(())
    });
    eids.insert("e3", e3.unwrap());
    Layers { t, eids }
}

/// Executes an IR query with no parameters.
pub fn run(view: &View<'_>, q: &IrQuery) -> QueryResult {
    view.execute_ir(q, &Params::new())
        .unwrap_or_else(|e| panic!("query failed: {e}\n{q}"))
}

/// Explains an IR query with no parameters.
pub fn explain(view: &View<'_>, q: &IrQuery) -> Explain {
    view.explain_ir(q, &Params::new())
        .unwrap_or_else(|e| panic!("explain failed: {e}\n{q}"))
}

/// Short text of a value: `v:` IRIs by local name, strings bare, eids `eN`.
pub fn short(v: &Value) -> String {
    match v {
        Value::Iri(s) => s
            .strip_prefix("urn:tiramemsu:v:")
            .map_or_else(|| format!("<{s}>"), str::to_string),
        Value::Str(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Double(x) => format!("{x}"),
        Value::Stmt(e) => format!("{e}"),
        Value::Tx(t) => format!("{t}"),
        Value::Bool(b) => b.to_string(),
        Value::DateTime { .. } => other_lexical(v).replace(".000", ""),
        other => other.lexical(),
    }
}

/// Short text of a cell (`-` for missing).
pub fn cell(c: &Option<ResultValue>) -> String {
    match c {
        None => "-".to_string(),
        Some(ResultValue::Term(v)) => short(v),
        Some(ResultValue::List(l)) => {
            format!("[{}]", l.iter().map(cell).collect::<Vec<_>>().join(","))
        }
        Some(other) => format!("{other:?}"),
    }
}

/// The rows as short text, in result order.
pub fn rows(r: &QueryResult) -> Vec<Vec<String>> {
    r.rows
        .iter()
        .map(|row| row.iter().map(cell).collect())
        .collect()
}

/// The rows as short text, sorted (a multiset comparison).
pub fn sorted(r: &QueryResult) -> Vec<Vec<String>> {
    let mut v = rows(r);
    v.sort();
    v
}

/// Builds expected rows from string slices.
pub fn expect(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect()
}

/// The column names of a result.
pub fn cols(r: &QueryResult) -> Vec<String> {
    r.columns.iter().map(|c| c.name().to_string()).collect()
}

fn other_lexical(v: &Value) -> String {
    v.lexical()
}
