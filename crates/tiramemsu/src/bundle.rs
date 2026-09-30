//! Text forms of a fact bundle: the versioned JSON format and RDF 1.2 N-Triples.
//!
//! The [`Bundle`] value lives in `tm-core`, which has no JSON dependency, so its
//! formats are an extension trait here, like [`crate::TxCypher`] for `Tx`.

use serde_json::{json, Map, Value as J};
use tm_core::{vocab, BTerm, Bundle, BundleStatement, Error, Position, Result, Valid, Value};
use tm_sparql::results::{nt, term::render, RdfTerm, RdfTriple};

/// The format string of the JSON form this build writes and reads.
pub const BUNDLE_FORMAT: &str = "tiramemsu-bundle/1";

/// The JSON and N-Triples forms of a [`Bundle`].
///
/// The JSON form is versioned and round-trips exactly:
///
/// ```json
/// { "format": "tiramemsu-bundle/1", "root": 1, "statements": [
///   { "id": 0, "s": {"iri": "…alice"}, "p": "…worksAt", "o": {"iri": "…acme"}, "validFrom": 1704067200000 },
///   { "id": 1, "s": {"ref": 0}, "p": "…confidence", "o": {"lex": "0.8", "datatype": "…#decimal"} } ] }
/// ```
///
/// A term is `{"iri"}`, `{"ref": id}` (a statement of the bundle), `{"blank": label}`
/// (an anonymous node), `{"lex", "datatype"}` or `{"lex", "lang"}`. `validFrom` and
/// `validTo` are epoch milliseconds, absent when unbounded. N-Triples is an export
/// for interchange; only the JSON form is read back.
///
/// ```
/// # use tiramemsu::*;
/// # let dir = tempfile::tempdir().unwrap();
/// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
/// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
/// let r = db.transact(TxOptions::default(), |tx| {
///     let job = tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?.eid();
///     tx.assert(job, v("source"), Value::str("chat"), Valid::ALWAYS)?;
///     Ok(())
/// })?;
/// let bundle = db.now().bundle(r.asserted[0])?;
/// let json = bundle.to_json();
/// assert_eq!(json["format"], "tiramemsu-bundle/1");
/// assert_eq!(Bundle::from_json(&json)?, bundle);
/// assert!(bundle.to_ntriples().contains("<http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies>"));
/// # Ok::<(), Error>(())
/// ```
// @lat: [[data-model#Fact Bundles#Bundle Formats]]
pub trait BundleFormat: Sized {
    /// The JSON form (`tiramemsu-bundle/1`).
    fn to_json(&self) -> J;

    /// Reads the JSON form back and checks the bundle's structure.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTerm`] for another format string, a missing or malformed
    /// field, or a bundle that fails [`Bundle::check`].
    fn from_json(j: &J) -> Result<Self>;

    /// RDF 1.2 N-Triples: each statement's triple, its reifier
    /// `_:s<id> rdf:reifies <<( s p o )>>`, and `tm:validFrom` / `tm:validTo` on the
    /// reifier when valid time is bounded. A statement in subject or object position
    /// is its reifier, so a layer is an annotation triple on the reifier; an
    /// anonymous node is `_:n<label>`.
    fn to_ntriples(&self) -> String;
}

fn bad(reason: impl Into<String>) -> Error {
    Error::InvalidTerm {
        position: Position::Value,
        reason: format!("bundle JSON: {}", reason.into()),
    }
}

fn term_json(t: &BTerm) -> J {
    match t {
        BTerm::Stmt(r) => json!({ "ref": r }),
        BTerm::Node(n) => json!({ "blank": n }),
        BTerm::Value(Value::LangStr { lex, lang }) => json!({ "lex": lex, "lang": lang }),
        BTerm::Value(v) => match v.datatype() {
            Some(dt) => json!({ "lex": v.lexical(), "datatype": dt }),
            // IRIs; a local id (node, statement, ...) is written as its skolem IRI,
            // which `Bundle::check` and import reject
            None => json!({ "iri": v.lexical() }),
        },
    }
}

fn term_from_json(j: &J, what: &str) -> Result<BTerm> {
    let o = j
        .as_object()
        .ok_or_else(|| bad(format!("{what} is not a term object")))?;
    let text = |k: &str| o.get(k).and_then(J::as_str);
    let id = |k: &str| -> Result<Option<u32>> {
        match o.get(k) {
            None => Ok(None),
            Some(n) => n
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| bad(format!("{what}.{k} is not a 32-bit id"))),
        }
    };
    if let Some(r) = id("ref")? {
        return Ok(BTerm::Stmt(r));
    }
    if let Some(n) = id("blank")? {
        return Ok(BTerm::Node(n));
    }
    if let Some(iri) = text("iri") {
        return Ok(BTerm::Value(Value::iri(iri)));
    }
    if let Some(lex) = text("lex") {
        return Ok(BTerm::Value(match (text("lang"), text("datatype")) {
            (Some(lang), None) => Value::literal(lex, None, Some(lang)),
            (None, Some(dt)) => Value::literal(lex, Some(dt), None),
            _ => {
                return Err(bad(format!(
                    "{what} needs exactly one of datatype and lang"
                )))
            }
        }));
    }
    Err(bad(format!("{what} is not a term: {j}")))
}

fn statement_json(st: &BundleStatement) -> J {
    let mut o = Map::new();
    o.insert("id".into(), json!(st.local));
    o.insert("s".into(), term_json(&st.s));
    o.insert(
        "p".into(),
        match &st.p {
            Value::Iri(p) => json!(p),
            other => json!(other.lexical()),
        },
    );
    o.insert("o".into(), term_json(&st.o));
    if let Some(f) = st.valid.from {
        o.insert("validFrom".into(), json!(f));
    }
    if let Some(t) = st.valid.to {
        o.insert("validTo".into(), json!(t));
    }
    J::Object(o)
}

fn statement_from_json(j: &J, k: usize) -> Result<BundleStatement> {
    let what = format!("statements[{k}]");
    let o = j
        .as_object()
        .ok_or_else(|| bad(format!("{what} is not an object")))?;
    let local = o
        .get("id")
        .and_then(J::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| bad(format!("{what}.id is not a 32-bit id")))?;
    let p = o
        .get("p")
        .and_then(J::as_str)
        .ok_or_else(|| bad(format!("{what}.p is not an IRI string")))?;
    let field = |f: &str| {
        o.get(f)
            .ok_or_else(|| bad(format!("{what}.{f} is missing")))
    };
    let time = |f: &str| -> Result<Option<i64>> {
        match o.get(f) {
            None | Some(J::Null) => Ok(None),
            Some(t) => t
                .as_i64()
                .map(Some)
                .ok_or_else(|| bad(format!("{what}.{f} is not epoch milliseconds"))),
        }
    };
    Ok(BundleStatement {
        local,
        s: term_from_json(field("s")?, &format!("{what}.s"))?,
        p: Value::iri(p),
        o: term_from_json(field("o")?, &format!("{what}.o"))?,
        valid: Valid {
            from: time("validFrom")?,
            to: time("validTo")?,
        },
    })
}

fn rdf_term(t: &BTerm) -> RdfTerm {
    match t {
        BTerm::Stmt(r) => RdfTerm::Blank(format!("s{r}")),
        BTerm::Node(n) => RdfTerm::Blank(format!("n{n}")),
        BTerm::Value(v) => render(v),
    }
}

impl BundleFormat for Bundle {
    fn to_json(&self) -> J {
        json!({
            "format": BUNDLE_FORMAT,
            "root": self.root,
            "statements": self.statements.iter().map(statement_json).collect::<Vec<_>>(),
        })
    }

    fn from_json(j: &J) -> Result<Bundle> {
        match j.get("format").and_then(J::as_str) {
            Some(BUNDLE_FORMAT) => {}
            Some(other) => return Err(bad(format!("unknown format {other:?}"))),
            None => return Err(bad("`format` is missing")),
        }
        let root = j
            .get("root")
            .and_then(J::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| bad("`root` is not a 32-bit id"))?;
        let statements = j
            .get("statements")
            .and_then(J::as_array)
            .ok_or_else(|| bad("`statements` is not a list"))?
            .iter()
            .enumerate()
            .map(|(k, s)| statement_from_json(s, k))
            .collect::<Result<Vec<_>>>()?;
        let b = Bundle { root, statements };
        b.check()?;
        Ok(b)
    }

    fn to_ntriples(&self) -> String {
        let reifies = RdfTerm::Iri(format!("{}reifies", vocab::RDF));
        let bound = |name: &str, ms: i64| {
            (
                RdfTerm::Iri(format!("{}{name}", vocab::TM)),
                render(&Value::DateTime { ms, tz: Some(0) }),
            )
        };
        let mut out = Vec::with_capacity(self.statements.len() * 2);
        for st in &self.statements {
            let triple = RdfTriple {
                s: rdf_term(&st.s),
                p: render(&st.p),
                o: rdf_term(&st.o),
            };
            let reifier = RdfTerm::Blank(format!("s{}", st.local));
            out.push(triple.clone());
            out.push(RdfTriple {
                s: reifier.clone(),
                p: reifies.clone(),
                o: RdfTerm::Triple(Box::new(triple)),
            });
            let times = [
                st.valid.from.map(|ms| bound("validFrom", ms)),
                st.valid.to.map(|ms| bound("validTo", ms)),
            ];
            for (p, o) in times.into_iter().flatten() {
                out.push(RdfTriple {
                    s: reifier.clone(),
                    p,
                    o,
                });
            }
        }
        nt::write(&out)
    }
}
