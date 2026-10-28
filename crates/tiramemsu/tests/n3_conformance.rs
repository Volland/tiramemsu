//! Conformance of the statement mapping with eyeron's example corpus (OpenSpec
//! change `add-n3-reasoning`, task 6.1): each example's facts are loaded into a
//! store as statements, its rules are passed as N3 text, and `View::reason`
//! must conclude what eyeron concludes. The subset and what was left out are
//! described in `tests/n3_conformance/README.md`.
#![cfg(feature = "n3")]

mod n3_common;

use std::collections::BTreeSet;
use std::path::Path;

use n3_common::backends;
use tiramemsu::n3_engine::eyeron::{self, ast, Document, Term as NTerm, Triple as NTriple};
use tiramemsu::n3_engine::{from_n3, value_of};
use tiramemsu::*;

/// The examples of `tests/n3_conformance/`.
const EXAMPLES: &[&str] = &[
    "access-control-policy",
    "aggregation",
    "ancestor",
    "backward",
    "backward-counting",
    "backward-recursion",
    "backward-rule-matching",
    "base-and-literals",
    "bmi",
    "check-unsafe",
    "cyclic-reachability",
    "deep-taxonomy-100",
    "defeasable-logic",
    "dog",
    "equals",
    "expression-eval",
    "family",
    "family-cousins",
    "fibonacci",
    "fuse",
    "graph-includes",
    "liar",
    "list-builtins-tests",
    "list-map",
    "log-for-all-in",
    "negation",
    "nixon-diamond",
    "owl-all-values-from",
    "owl-equivalent-classes",
    "owl-functional-property",
    "owl-has-value",
    "owl-inverse-properties",
    "owl-max-cardinality",
    "owl-same-as",
    "owl-some-values-from",
    "owl-symmetric-property",
    "owl-transitive-property",
    "policy",
    "property-paths",
    "query",
    "rdfs-domain",
    "rdfs-range",
    "rdfs-subclass-closure",
    "rdfs-subproperty-closure",
    "road-paths",
    "snaf",
    "socrates",
    "stratified-negation",
    "string-builtins-tests",
    "time",
    "turtle-shortcuts",
    "unicode-and-signed-numbers",
    "variable-predicate-dependency",
    "witch",
];

fn key(t: &NTriple) -> String {
    format!("{} {} {}", from_n3(&t.s), from_n3(&t.p), from_n3(&t.o))
}

/// The example's rules as N3 text, without its facts.
fn rules_text(doc: &Document) -> String {
    let mut out = String::new();
    for (p, iri) in &doc.prefixes {
        out.push_str(&format!("@prefix {p}: <{iri}> .\n"));
    }
    let formula = |g: &eyeron::Graph| NTerm::Formula(g.clone());
    for r in &doc.rules {
        if r.is_fuse {
            let body = eyeron::printing::term_to_n3_object(&formula(&r.premise), &doc.prefixes);
            out.push_str(&format!("{body} => false .\n"));
            continue;
        }
        let t = if r.is_query {
            NTriple::new(
                formula(&r.premise),
                NTerm::iri(ast::LOG_QUERY),
                formula(&r.conclusion),
            )
        } else if r.is_forward {
            NTriple::new(
                formula(&r.premise),
                NTerm::iri(ast::LOG_IMPLIES),
                formula(&r.conclusion),
            )
        } else {
            NTriple::new(
                formula(&r.conclusion),
                NTerm::iri(ast::LOG_IMPLIED_BY),
                formula(&r.premise),
            )
        };
        out.push_str(&eyeron::printing::triple_to_n3(&doc.prefixes, &t));
        out.push_str(" .\n");
    }
    out
}

fn check(dir: &Path, name: &str, backend: &str, db: &Db) {
    let src = std::fs::read_to_string(dir.join(format!("{name}.n3"))).unwrap();
    let golden = std::fs::read_to_string(dir.join(format!("output/{name}.n3"))).unwrap();
    let doc = eyeron::parse_n3(&src, None).unwrap();
    let gold = eyeron::parse_n3(&golden, None).unwrap();
    db.transact(TxOptions::default(), |tx| {
        for t in &doc.facts {
            let (s, p, o) = (
                value_of(&t.s).expect("storable"),
                value_of(&t.p).expect("storable"),
                value_of(&t.o).expect("storable"),
            );
            tx.assert(s, p, o, Valid::ALWAYS)?;
        }
        Ok(())
    })
    .unwrap_or_else(|e| panic!("{name} on {backend}: loading: {e}"));
    let r = db
        .now()
        .reason(Rules::Text(rules_text(&doc)), &ReasonOptions::default())
        .unwrap_or_else(|e| panic!("{name} on {backend}: {e}"));
    assert_eq!(r.status, ReasonStatus::Complete, "{name} on {backend}");
    let facts: BTreeSet<String> = doc.facts.iter().map(key).collect();
    let expected: BTreeSet<String> = gold
        .facts
        .iter()
        .map(key)
        .filter(|k| !facts.contains(k))
        .collect();
    let got: BTreeSet<String> = r
        .conclusions
        .iter()
        .map(|c| format!("{} {} {}", c.s, c.p, c.o))
        .collect();
    assert_eq!(got, expected, "{name} on {backend}");
    // a conclusion is reported once, and its premises are loaded statements or
    // earlier conclusions
    assert_eq!(got.len(), r.conclusions.len(), "{name}");
    for (i, c) in r.conclusions.iter().enumerate() {
        for p in &c.proof.premises {
            if let Premise::Conclusion(j) = p {
                assert!(*j < i, "{name}: conclusion {i} uses {j}");
            }
        }
    }
}

// @tg: verifies:: [[openspec:n3-reasoning#Statements in N3]] {since: "2026-10-28", level: integration}
#[test]
fn eyeron_examples_through_the_store() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/n3_conformance");
    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            (p.extension()? == "n3").then_some(stem)
        })
        .collect();
    assert_eq!(
        on_disk,
        EXAMPLES
            .iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>(),
        "every copied example is checked"
    );
    for name in EXAMPLES {
        for (backend, _dir, db) in backends() {
            check(&dir, name, backend, &db);
        }
    }
}
