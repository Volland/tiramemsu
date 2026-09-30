//! Shared fixtures for the Cypher integration tests.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use tiramemsu::*;

/// `v:local` as a value.
pub fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// `urn:tiramemsu:v:local`.
pub fn vi(local: &str) -> String {
    format!("urn:tiramemsu:v:{local}")
}

/// A plain string value.
pub fn sv(text: &str) -> Value {
    Value::str(text)
}

/// The rdf:type IRI as a value.
pub fn rdf_type() -> Value {
    Value::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
}

pub fn s(x: &str) -> CypherValue {
    CypherValue::String(x.to_string())
}
pub fn i(x: i64) -> CypherValue {
    CypherValue::Integer(x)
}
pub fn f(x: f64) -> CypherValue {
    CypherValue::Float(x)
}
pub fn b(x: bool) -> CypherValue {
    CypherValue::Boolean(x)
}
pub fn null() -> CypherValue {
    CypherValue::Null
}
pub fn list(xs: Vec<CypherValue>) -> CypherValue {
    CypherValue::List(xs)
}
pub fn map(xs: &[(&str, CypherValue)]) -> CypherValue {
    CypherValue::Map(xs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}
/// A node reference usable as a parameter.
pub fn node_ref(t: Value) -> CypherValue {
    CypherValue::Node(Box::new(NodeValue {
        element_id: t.lexical(),
        term: t,
        labels: vec![],
        properties: BTreeMap::new(),
    }))
}

pub use tiramemsu::cypher_frontend::NodeValue;

/// The element id of a node or relationship value.
pub fn eid_of(v: &CypherValue) -> String {
    match v {
        CypherValue::Node(n) => n.element_id.clone(),
        CypherValue::Relationship(r) => r.element_id.clone(),
        other => panic!("not an entity: {other:?}"),
    }
}

/// The element id with the `urn:tiramemsu:v:` prefix stripped.
pub fn short(v: &CypherValue) -> String {
    eid_of(v).trim_start_matches("urn:tiramemsu:v:").to_string()
}

pub fn no_params() -> CypherParams {
    CypherParams::new()
}

pub fn params(pairs: &[(&str, CypherValue)]) -> CypherParams {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
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
            dir.path().join("c.db"),
            OpenOptions {
                clock: clock.clone(),
                ..OpenOptions::default()
            },
        )
        .expect("open");
        T { dir, db, clock }
    }

    pub fn tx<F: FnOnce(&mut Tx<'_>) -> Result<()>>(&self, f: F) -> TxReport {
        self.db.transact(TxOptions::default(), f).expect("commit")
    }

    /// Asserts triples in one transaction.
    pub fn assert(&self, triples: &[(Value, Value, Value)]) -> TxReport {
        self.tx(|tx| {
            for (a, b, c) in triples {
                tx.assert(a, b, c, Valid::ALWAYS)?;
            }
            Ok(())
        })
    }

    pub fn advance_to(&self, t: u64) {
        while self.last_t() < t {
            self.tx(|_| Ok(()));
        }
        assert_eq!(self.last_t(), t, "already past tx {t}");
    }

    pub fn last_t(&self) -> u64 {
        self.db
            .read_sql("SELECT value FROM meta WHERE key = 'last_t'")
            .unwrap()[0][0]
            .as_i64()
            .unwrap() as u64
    }

    /// Reads on the now view.
    pub fn q(&self, text: &str) -> CypherResult {
        self.db
            .now()
            .cypher(text, &no_params())
            .unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    pub fn qp(&self, text: &str, p: &CypherParams) -> CypherResult {
        self.db
            .now()
            .cypher(text, p)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    pub fn qerr(&self, text: &str) -> Error {
        self.db.now().cypher(text, &no_params()).expect_err(text)
    }

    /// One value of a one-row, one-column result.
    pub fn one(&self, text: &str) -> CypherValue {
        let r = self.q(text);
        assert_eq!(r.rows.len(), 1, "{text}: {:?}", r.rows);
        r.rows[0][0].clone()
    }

    /// Writes through the write entry point.
    pub fn w(&self, text: &str) -> CypherResult {
        self.db
            .cypher_write(TxOptions::default(), text, &no_params())
            .unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    pub fn wp(&self, text: &str, p: &CypherParams) -> CypherResult {
        self.db
            .cypher_write(TxOptions::default(), text, p)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    pub fn werr(&self, text: &str) -> Error {
        self.db
            .cypher_write(TxOptions::default(), text, &no_params())
            .expect_err(text)
    }

    /// Live statements as `(s, p, o)` rendered with `v:` names.
    pub fn live(&self) -> Vec<(String, String, String)> {
        self.db
            .now()
            .triples(None, None, None)
            .unwrap()
            .into_iter()
            .map(|t| {
                let d = |id| self.db.now().decode(id).unwrap();
                let r = |x: Value| match x {
                    Value::Iri(i) => i.trim_start_matches("urn:tiramemsu:v:").to_string(),
                    other => other.lexical(),
                };
                (r(d(t.s)), r(d(t.p)), r(d(t.o)))
            })
            .collect()
    }
}
