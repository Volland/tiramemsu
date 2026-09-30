//! Shared fixtures for the SPARQL integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use tiramemsu::*;

/// `v:local` as a value.
pub fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// A plain string.
pub fn s(text: &str) -> Value {
    Value::str(text)
}

/// A temporary database with a manual clock (1 ms per transaction).
pub struct T {
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub clock: Arc<ManualClock>,
}

impl T {
    pub fn new() -> T {
        let dir = tempfile::tempdir().expect("tempdir");
        let clock = Arc::new(ManualClock::new(1_000));
        let db = Db::open(
            dir.path().join("q.db"),
            OpenOptions {
                clock: clock.clone(),
                ..OpenOptions::default()
            },
        )
        .expect("open");
        T { dir, db, clock }
    }

    /// Runs a transaction that must commit.
    pub fn tx<F: FnOnce(&mut Tx<'_>) -> Result<()>>(&self, f: F) -> TxReport {
        self.db.transact(TxOptions::default(), f).expect("commit")
    }

    /// Asserts triples `(s, p, o)` in one transaction.
    pub fn assert(&self, triples: &[(Value, Value, Value)]) -> TxReport {
        self.tx(|tx| {
            for (a, b, c) in triples {
                tx.assert(a, b, c, Valid::ALWAYS)?;
            }
            Ok(())
        })
    }

    /// Commits empty transactions until the last `t` is `t`.
    pub fn advance_to(&self, t: u64) {
        loop {
            let n = self.last_t();
            if n >= t {
                assert_eq!(n, t, "already past tx {t}");
                return;
            }
            self.tx(|_| Ok(()));
        }
    }

    pub fn last_t(&self) -> u64 {
        self.db
            .read_sql("SELECT value FROM meta WHERE key = 'last_t'")
            .unwrap()[0][0]
            .as_i64()
            .unwrap() as u64
    }

    /// Retracts every live `(s, p, o)` in one transaction.
    pub fn retract(&self, a: &Value, b: &Value, c: &Value) -> TxReport {
        self.tx(|tx| {
            let (a, b, c) = (tx.encode(a)?, tx.encode(b)?, tx.encode(c)?);
            tx.retract_matching(Some(a), Some(b), Some(c))?;
            Ok(())
        })
    }

    /// A SELECT on the current view.
    pub fn sel(&self, q: &str) -> Solutions {
        sel_on(&self.db.now(), q)
    }

    /// An ASK on the current view.
    pub fn ask(&self, q: &str) -> bool {
        match self
            .db
            .now()
            .sparql(q)
            .unwrap_or_else(|e| panic!("{q}: {e}"))
        {
            SparqlResult::Boolean(b) => b,
            other => panic!("not a boolean: {other:?}"),
        }
    }

    /// The column `var` of a SELECT on the current view.
    pub fn col(&self, q: &str, var: &str) -> Vec<Option<Value>> {
        let s = self.sel(q);
        let c = s
            .col(var)
            .unwrap_or_else(|| panic!("no column {var} in {:?}", s.vars));
        s.rows.iter().map(|r| r[c].clone()).collect()
    }

    /// Runs an update request on the current view.
    pub fn upd(&self, q: &str) -> TxReport {
        match self
            .db
            .now()
            .sparql(q)
            .unwrap_or_else(|e| panic!("{q}: {e}"))
        {
            SparqlResult::Update(r) => r,
            other => panic!("not an update report: {other:?}"),
        }
    }

    /// The live `(s, p, o)` statements as values, in eid order.
    pub fn live(&self) -> Vec<(Value, Value, Value)> {
        let view = self.db.now();
        view.triples(None, None, None)
            .unwrap()
            .iter()
            .map(|t| {
                (
                    view.decode(t.s).unwrap(),
                    view.decode(t.p).unwrap(),
                    view.decode(t.o).unwrap(),
                )
            })
            .collect()
    }

    /// True when `(s, p, o)` is live.
    pub fn has(&self, a: &Value, b: &Value, c: &Value) -> bool {
        self.live()
            .iter()
            .any(|(x, y, z)| x == a && y == b && z == c)
    }

    /// The error of a request on the current view.
    pub fn err(&self, q: &str) -> Error {
        match self.db.now().sparql(q) {
            Ok(r) => panic!("expected an error for {q}, got {r:?}"),
            Err(e) => e,
        }
    }
}

pub fn sel_on(view: &View<'_>, q: &str) -> Solutions {
    match view.sparql(q).unwrap_or_else(|e| panic!("{q}: {e}")) {
        SparqlResult::Solutions(s) => s,
        other => panic!("not solutions: {other:?}"),
    }
}

/// `Some(value)` cells for a list of values.
pub fn some(vals: &[Value]) -> Vec<Option<Value>> {
    vals.iter().cloned().map(Some).collect()
}

pub fn int(n: i64) -> Value {
    Value::Int(n)
}

pub fn dt(lex: &str) -> Value {
    Value::literal(lex, Some(vocab::XSD_DATETIME), None)
}

#[track_caller]
pub fn assert_unsupported(e: Error, feature: &str) {
    match e {
        Error::Unsupported { feature: f } => assert_eq!(f, feature),
        other => panic!("expected Unsupported({feature}), got {other:?}"),
    }
}

#[track_caller]
pub fn assert_parse(e: Error) -> (Option<Span>, String) {
    match e {
        Error::Parse { dialect, span, msg } => {
            assert_eq!(dialect, Dialect::Sparql);
            (span, msg)
        }
        other => panic!("expected Parse, got {other:?}"),
    }
}
