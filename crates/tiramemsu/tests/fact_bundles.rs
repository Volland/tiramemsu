//! Spec `fact-bundles` through the facade: the JSON and N-Triples forms, and a bundle
//! that travels between two databases as JSON text.

use serde_json::json;
use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn open(dir: &tempfile::TempDir, name: &str) -> Db {
    Db::open(dir.path().join(name), OpenOptions::default()).unwrap()
}

fn every_kind() -> Bundle {
    let st = |local, s, p: &str, o, valid| BundleStatement {
        local,
        s,
        p: v(p),
        o,
        valid,
    };
    let val = BTerm::Value;
    Bundle {
        root: 0,
        statements: vec![
            st(
                0,
                val(v("alice")),
                "worksAt",
                val(v("acme")),
                Valid::between(1_577_836_800_000, 1_704_067_200_000),
            ),
            st(
                1,
                BTerm::Stmt(0),
                "confidence",
                val(Value::Double(0.8)),
                Valid::ALWAYS,
            ),
            st(
                2,
                BTerm::Stmt(1),
                "source",
                val(Value::str("a chat message")),
                Valid::from(-86_400_000),
            ),
            st(
                3,
                BTerm::Stmt(0),
                "label",
                val(Value::LangStr {
                    lex: "Arbeit".into(),
                    lang: "de".into(),
                }),
                Valid::until(0),
            ),
            st(
                4,
                BTerm::Stmt(0),
                "count",
                val(Value::Int(-42)),
                Valid::ALWAYS,
            ),
            st(
                5,
                BTerm::Stmt(0),
                "seenAt",
                val(Value::literal(
                    "2026-03-01T12:00:00+02:00",
                    Some(vocab::XSD_DATETIME),
                    None,
                )),
                Valid::ALWAYS,
            ),
            st(
                6,
                BTerm::Stmt(0),
                "on",
                val(Value::Date(20_000)),
                Valid::ALWAYS,
            ),
            st(
                7,
                BTerm::Stmt(0),
                "amount",
                val(Value::literal("12.50", Some(vocab::XSD_DECIMAL), None)),
                Valid::ALWAYS,
            ),
            st(
                8,
                BTerm::Stmt(0),
                "ok",
                val(Value::Bool(true)),
                Valid::ALWAYS,
            ),
            st(
                9,
                BTerm::Stmt(0),
                "big",
                val(Value::big_integer("123456789012345678901234567890")),
                Valid::ALWAYS,
            ),
            st(
                10,
                BTerm::Stmt(0),
                "raw",
                val(Value::literal("x", Some("urn:dt"), None)),
                Valid::ALWAYS,
            ),
            st(
                11,
                BTerm::Node(0),
                "supportedBy",
                BTerm::Stmt(0),
                Valid::ALWAYS,
            ),
            st(12, BTerm::Node(0), "knows", BTerm::Node(1), Valid::ALWAYS),
        ],
    }
}

// @lat: [[tests#Fact Bundles#Bundle JSON Round Trip]]
#[test]
fn json_round_trip_is_exact_and_stable() {
    let b = every_kind();
    let j = b.to_json();
    assert_eq!(j["format"], BUNDLE_FORMAT);
    assert_eq!(j["statements"][1]["s"], json!({ "ref": 0 }));
    assert_eq!(j["statements"][11]["s"], json!({ "blank": 0 }));
    assert_eq!(
        j["statements"][3]["o"],
        json!({ "lex": "Arbeit", "lang": "de" })
    );
    assert_eq!(j["statements"][0]["validFrom"], json!(1_577_836_800_000i64));
    assert!(j["statements"][1].get("validFrom").is_none());
    let text = j.to_string();
    let back = Bundle::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(back, b);
    assert_eq!(back.to_json().to_string(), text);

    // another version, a missing field, a malformed term, a dangling reference
    let mut other = j.clone();
    other["format"] = json!("tiramemsu-bundle/2");
    let broken = [
        other,
        json!({ "format": BUNDLE_FORMAT, "statements": [] }),
        json!({ "format": BUNDLE_FORMAT, "root": 0, "statements": [{ "id": 0, "s": 1, "p": "urn:p", "o": {"iri": "urn:o"} }] }),
        json!({ "format": BUNDLE_FORMAT, "root": 0, "statements": [{ "id": 0, "s": {"ref": 3}, "p": "urn:p", "o": {"iri": "urn:o"} }] }),
        json!({ "format": BUNDLE_FORMAT, "root": 0, "statements": [{ "id": 0, "s": {"iri": "urn:s"}, "p": "urn:p", "o": {"lex": "x", "lang": "en", "datatype": "urn:d"} }] }),
        json!({ "format": BUNDLE_FORMAT, "root": 0, "statements": [{ "id": 0, "s": {"iri": "urn:tiramemsu:node:1"}, "p": "urn:p", "o": {"iri": "urn:o"} }] }),
    ];
    for j in &broken {
        assert!(
            matches!(Bundle::from_json(j), Err(Error::InvalidTerm { .. })),
            "accepted {j}"
        );
    }
}

// @lat: [[tests#Fact Bundles#Bundle N-Triples Is RDF 1.2]]
#[test]
fn ntriples_parse_as_rdf12() {
    let b = every_kind();
    let text = b.to_ntriples();
    let parsed: Vec<_> = oxttl::NTriplesParser::new()
        .for_reader(text.as_bytes())
        .collect::<Result<_, _>>()
        .expect("valid RDF 1.2 N-Triples");
    // one triple and one reifier per statement, plus four valid-time bounds
    assert_eq!(parsed.len(), 2 * b.statements.len() + 4);
    let alice = "<urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> <urn:tiramemsu:v:acme>";
    assert!(text.contains(&format!("{alice} .\n")));
    assert!(text.contains(&format!(
        "_:s0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( {alice} )>> .\n"
    )));
    // a layer is an annotation triple on the reifier
    assert!(text.contains("_:s0 <urn:tiramemsu:v:confidence> "));
    assert!(text.contains(
        "_:s0 <urn:tiramemsu:tm:validFrom> \"2020-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n"
    ));
    assert!(text.contains("_:n0 <urn:tiramemsu:v:knows> _:n1 .\n"));
    assert!(!text.contains("urn:tiramemsu:node:"));
}

// @lat: [[tests#Fact Bundles#Bundle Travels As JSON Text]]
#[test]
fn a_belief_travels_between_databases_as_json() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (open(&dir, "a.db"), open(&dir, "b.db"));
    let r = a
        .transact(TxOptions::default(), |tx| {
            let job = tx
                .assert(
                    v("alice"),
                    v("worksAt"),
                    v("acme"),
                    Valid::from(1_577_836_800_000),
                )?
                .eid();
            tx.assert(job, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
            let belief = tx
                .assert(v("belief9"), v("supportedBy"), job, Valid::ALWAYS)?
                .eid();
            tx.add_to_graph(belief, v("agent7"), AssertOpts::default())?;
            Ok(())
        })
        .unwrap();
    let belief = r.asserted[2];
    let text = a.now().bundle(belief).unwrap().to_json().to_string();
    let bundle = Bundle::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
    // the belief, the fact it relies on, and the membership (not the confidence)
    assert_eq!(bundle.statements.len(), 3);
    let mut report = None;
    b.transact(TxOptions::default(), |tx| {
        report = Some(tx.import_bundle(&bundle)?);
        Ok(())
    })
    .unwrap();
    let report = report.unwrap();
    // the layered query runs where the SPARQL front end is compiled; the JSON
    // round trip and the import above need no query front end
    #[cfg(feature = "sparql")]
    {
        let rows = b
            .now()
            .sparql(
                "SELECT ?who ?c WHERE { GRAPH v:agent7 { ?who v:supportedBy ?f } \
                 ?f sys:subject v:alice ; sys:object ?c }",
            )
            .unwrap();
        let SparqlResult::Solutions(rows) = rows else {
            panic!("not a select")
        };
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.get(0, "c"), Some(&v("acme")));
    }
    assert_eq!(b.now().bundle(report.root).unwrap(), bundle);
    // importing it again inside a speculation asserts nothing
    let asserted = b
        .with(
            |tx| tx.import_bundle(&bundle).map(|_| ()),
            |view| Ok(view.triples(None, None, None)?.len()),
        )
        .unwrap();
    assert_eq!(asserted, 3);
}
