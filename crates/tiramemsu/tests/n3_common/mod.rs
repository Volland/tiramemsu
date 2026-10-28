//! Shared fixtures for the N3 reasoning tests (feature `n3`).
#![allow(dead_code)]

use tiramemsu::*;

/// `v:local` as a value.
pub fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// `rdf:type`.
pub fn a() -> Value {
    Value::iri(vocab::RDF_TYPE)
}

/// Midnight UTC on the first of January of `year`, in epoch ms.
pub fn year(year: i32) -> i64 {
    date(&format!("{year:04}-01-01"))
}

/// Midnight UTC of an ISO date, in epoch ms.
pub fn date(iso: &str) -> i64 {
    value::parse_datetime(&format!("{iso}T00:00:00Z"))
        .expect("a date")
        .0
}

/// A fresh database on every backend this build has: SQLite, and redb with
/// the `redb` feature.
pub fn backends() -> Vec<(&'static str, tempfile::TempDir, Db)> {
    let mut out = Vec::new();
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Db::open(dir.path().join("n3.db"), OpenOptions::default()).expect("open");
    out.push(("sqlite", dir, db));
    #[cfg(feature = "redb")]
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::create(
            dir.path().join("n3.redb"),
            Backend::Redb,
            OpenOptions::default(),
        )
        .expect("create");
        out.push(("redb", dir, db));
    }
    out
}

/// Runs `f` on a fresh database of every backend.
pub fn each(mut f: impl FnMut(&str, &Db)) {
    for (name, _dir, db) in backends() {
        f(name, &db);
    }
}

/// Text rules.
pub fn text(n3: &str) -> Rules {
    Rules::Text(n3.to_string())
}

/// A conclusion's content as `s p o` text, comparable across runs.
pub fn content(c: &Conclusion) -> String {
    format!("{} {} {}", c.s, c.p, c.o)
}

/// `s p o` with `v:` values.
pub fn spo(s: &Value, p: &Value, o: &Value) -> String {
    format!("{s} {p} {o}")
}

/// The Socrates rule.
pub const MORTALITY: &str = "{ ?x a v:Human } => { ?x a v:Mortal } .";
