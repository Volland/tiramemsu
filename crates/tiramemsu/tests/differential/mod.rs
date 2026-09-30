//! The SPARQL half of the differential corpus (`lat.md/tests#Query#Differential
//! SPARQL Cypher`, task 11.4).
//!
//! Every case is a SPARQL query over the shared fixture with the result the
//! query must produce as a set of rows (columns in projection order, each cell
//! rendered by `cell`), which is the normal form for comparing with Cypher after
//! set-versus-bag normalisation. The Cypher harness of `add-cypher-frontend`
//! (M2b) adds the equivalent Cypher text per case and compares the same rows.
#![allow(dead_code)]

use std::collections::BTreeSet;

use tiramemsu::*;

/// One corpus entry.
pub struct Case {
    pub name: &'static str,
    pub sparql: &'static str,
    /// The rows as sets of rendered cells (`-` is unbound).
    pub rows: &'static [&'static [&'static str]],
}

/// alice/bob/carol, who works where, ages and names.
pub fn fixture(db: &Db) {
    let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    db.transact(TxOptions::default(), |tx| {
        for (s, p, o) in [
            ("alice", "worksAt", "acme"),
            ("bob", "worksAt", "acme"),
            ("carol", "worksAt", "initech"),
            ("acme", "locatedIn", "berlin"),
            ("initech", "locatedIn", "paris"),
            ("alice", "knows", "bob"),
            ("bob", "knows", "carol"),
        ] {
            tx.assert(v(s), v(p), v(o), Valid::ALWAYS)?;
        }
        for (s, n) in [("alice", 41), ("bob", 30), ("carol", 25)] {
            tx.assert(v(s), v("age"), Value::Int(n), Valid::ALWAYS)?;
            tx.assert(
                v(s),
                v("name"),
                Value::str(s[..1].to_uppercase() + &s[1..]),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    })
    .unwrap();
}

/// Renders a cell: local names of `v:` IRIs, plain lexical forms otherwise.
pub fn cell(v: &Option<Value>) -> String {
    match v {
        None => "-".to_string(),
        Some(Value::Iri(i)) => i.strip_prefix("urn:tiramemsu:v:").unwrap_or(i).to_string(),
        Some(other) => other.lexical(),
    }
}

/// The rows of a SPARQL result as a set.
pub fn rows(s: &Solutions) -> BTreeSet<Vec<String>> {
    s.rows
        .iter()
        .map(|r| r.iter().map(cell).collect())
        .collect()
}

pub const CASES: &[Case] = &[
    Case {
        name: "point lookup",
        sparql: "SELECT ?c WHERE { v:alice v:worksAt ?c }",
        rows: &[&["acme"]],
    },
    Case {
        name: "two-hop join",
        sparql: "SELECT ?p ?city WHERE { ?p v:worksAt ?c . ?c v:locatedIn ?city }",
        rows: &[
            &["alice", "berlin"],
            &["bob", "berlin"],
            &["carol", "paris"],
        ],
    },
    Case {
        name: "optional",
        sparql: "SELECT ?p ?k WHERE { ?p v:name ?n OPTIONAL { ?p v:knows ?k } }",
        rows: &[&["alice", "bob"], &["bob", "carol"], &["carol", "-"]],
    },
    Case {
        name: "filter",
        sparql: "SELECT ?p WHERE { ?p v:age ?a FILTER(?a > 28) }",
        rows: &[&["alice"], &["bob"]],
    },
    Case {
        name: "union",
        sparql: "SELECT ?x WHERE { { v:alice v:knows ?x } UNION { v:alice v:worksAt ?x } }",
        rows: &[&["bob"], &["acme"]],
    },
    Case {
        name: "count per group",
        sparql: "SELECT ?c (COUNT(?p) AS ?n) WHERE { ?p v:worksAt ?c } GROUP BY ?c",
        rows: &[&["acme", "2"], &["initech", "1"]],
    },
    Case {
        name: "distinct",
        sparql: "SELECT DISTINCT ?c WHERE { ?p v:worksAt ?c }",
        rows: &[&["acme"], &["initech"]],
    },
    Case {
        name: "order and limit",
        sparql: "SELECT ?p WHERE { ?p v:age ?a } ORDER BY DESC(?a) LIMIT 2",
        rows: &[&["alice"], &["bob"]],
    },
    Case {
        name: "not exists",
        sparql: "SELECT ?p WHERE { ?p v:name ?n FILTER NOT EXISTS { ?p v:knows ?k } }",
        rows: &[&["carol"]],
    },
    Case {
        name: "string function",
        sparql: "SELECT ?p (UCASE(?n) AS ?u) WHERE { ?p v:name ?n FILTER(?p = v:bob) }",
        rows: &[&["bob", "BOB"]],
    },
    Case {
        name: "statement identity",
        sparql: "SELECT (COUNT(?r) AS ?n) WHERE { v:alice v:knows v:bob ~ ?r }",
        rows: &[&["1"]],
    },
];
