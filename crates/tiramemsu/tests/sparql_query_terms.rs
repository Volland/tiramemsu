//! `sparql-query`: built-in functions, literal canonicalisation, blank nodes and
//! skolem IRIs, interim paths, parse errors and predeclared prefixes.
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

fn xsd(local: &str) -> String {
    format!("{}{local}", vocab::XSD)
}

// sparql-query "Built-in functions": String functions
#[test]
fn string_functions() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), s("Alice Smith"))]);
    let got = t.col(
        "SELECT (UCASE(STRBEFORE(?n, \" \")) AS ?f) WHERE { v:alice v:name ?n }",
        "f",
    );
    assert_eq!(got, some(&[s("ALICE")]));
    let r = t.sel(
        "SELECT (STRAFTER(?n, \" \") AS ?a) (SUBSTR(?n, 1, 5) AS ?b) (CONCAT(?n, \"!\", \"?\") AS ?c) \
         (STRLEN(?n) AS ?d) (REPLACE(?n, \"[aeiou]\", \"_\") AS ?e) (ENCODE_FOR_URI(?n) AS ?f) \
         (LCASE(?n) AS ?g) WHERE { v:alice v:name ?n }",
    );
    assert_eq!(r.get(0, "a"), Some(&s("Smith")));
    assert_eq!(r.get(0, "b"), Some(&s("Alice")));
    assert_eq!(r.get(0, "c"), Some(&s("Alice Smith!?")));
    assert_eq!(r.get(0, "d"), Some(&int(11)));
    assert_eq!(r.get(0, "e"), Some(&s("Al_c_ Sm_th")));
    assert_eq!(r.get(0, "f"), Some(&s("Alice%20Smith")));
    assert_eq!(r.get(0, "g"), Some(&s("alice smith")));
}

#[test]
fn term_constructors_and_tests() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), s("Alice"))]);
    let r = t.sel(
        "SELECT (STRLANG(?n, \"EN\") AS ?a) (LANG(STRLANG(?n, \"EN\")) AS ?l) (isBlank(?p) AS ?b) \
         (isIRI(?p) AS ?i) (IRI(\"http://example.org/x\") AS ?u) (STRDT(\"5\", <http://x/dt>) AS ?t) \
         (LANGMATCHES(\"en-GB\", \"en\") AS ?m) (DATATYPE(STRDT(\"5\", <http://x/dt>)) AS ?dt) \
         WHERE { ?p v:name ?n }",
    );
    assert_eq!(
        r.get(0, "a"),
        Some(&Value::LangStr {
            lex: "Alice".into(),
            lang: "en".into()
        })
    );
    assert_eq!(r.get(0, "l"), Some(&s("en")));
    assert_eq!(r.get(0, "b"), Some(&Value::Bool(false)));
    assert_eq!(r.get(0, "i"), Some(&Value::Bool(true)));
    assert_eq!(r.get(0, "u"), Some(&Value::iri("http://example.org/x")));
    assert_eq!(
        r.get(0, "t"),
        Some(&Value::Typed {
            lex: "5".into(),
            datatype: "http://x/dt".into()
        })
    );
    assert_eq!(r.get(0, "m"), Some(&Value::Bool(true)));
    assert_eq!(r.get(0, "dt"), Some(&Value::iri("http://x/dt")));
}

#[test]
fn numeric_functions_and_casts() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("x"), Value::Double(-2.5)),
        (v("a"), v("n"), int(-7)),
    ]);
    let r = t.sel(
        "SELECT (ABS(?n) AS ?abs) (CEIL(?x) AS ?ceil) (FLOOR(?x) AS ?floor) (ROUND(?x) AS ?round) \
         (xsd:integer(\"42\") AS ?i) (xsd:double(\"1.5\") AS ?d) (xsd:boolean(\"true\") AS ?b) \
         (xsd:string(?n) AS ?s) (xsd:integer(3.9) AS ?t) \
         WHERE { v:a v:x ?x . v:a v:n ?n }",
    );
    assert_eq!(r.get(0, "abs"), Some(&int(7)));
    assert_eq!(r.get(0, "ceil"), Some(&Value::Double(-2.0)));
    assert_eq!(r.get(0, "floor"), Some(&Value::Double(-3.0)));
    assert_eq!(r.get(0, "round"), Some(&Value::Double(-2.0)));
    assert_eq!(r.get(0, "i"), Some(&int(42)));
    assert_eq!(r.get(0, "d"), Some(&Value::Double(1.5)));
    assert_eq!(r.get(0, "b"), Some(&Value::Bool(true)));
    assert_eq!(r.get(0, "s"), Some(&s("-7")));
    assert_eq!(r.get(0, "t"), Some(&int(3)));
}

#[test]
fn date_functions_and_casts() {
    let t = T::new();
    t.assert(&[(v("e"), v("at"), dt("2026-09-01T14:05:06+02:00"))]);
    let r = t.sel(
        "SELECT (YEAR(?t) AS ?y) (MONTH(?t) AS ?mo) (DAY(?t) AS ?d) (HOURS(?t) AS ?h) \
         (MINUTES(?t) AS ?mi) (SECONDS(?t) AS ?s) (xsd:dateTime(\"2026-01-02T03:04:05Z\") AS ?c) \
         (xsd:date(\"2026-03-04\") AS ?dd) WHERE { v:e v:at ?t }",
    );
    assert_eq!(r.get(0, "y"), Some(&int(2026)));
    assert_eq!(r.get(0, "mo"), Some(&int(9)));
    assert_eq!(r.get(0, "d"), Some(&int(1)));
    assert_eq!(r.get(0, "h"), Some(&int(14)));
    assert_eq!(r.get(0, "mi"), Some(&int(5)));
    assert_eq!(r.get(0, "s"), Some(&int(6)));
    assert_eq!(r.get(0, "c"), Some(&dt("2026-01-02T03:04:05Z")));
    assert_eq!(
        r.get(0, "dd"),
        Some(&Value::literal("2026-03-04", Some(vocab::XSD_DATE), None))
    );
}

// sparql-query "Built-in functions": NOW is fixed per query
#[test]
fn now_is_fixed_per_query() {
    let t = T::new();
    t.clock.set(1_800_000_000_000);
    t.assert(&[(v("a"), v("p"), v("b"))]);
    let r = t.sel("SELECT (NOW() AS ?a) (NOW() AS ?b) WHERE { v:a v:p v:b }");
    assert_eq!(r.get(0, "a"), r.get(0, "b"));
    assert!(r.get(0, "a").is_some());
}

// sparql-query "Built-in functions": Unsupported function is named
#[test]
fn unsupported_function_is_named() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), s("Alice"))]);
    let e = t.err("SELECT (MD5(?n) AS ?h) WHERE { ?p v:name ?n }");
    assert_unsupported(e, "MD5");
}

// sparql-query "Built-in functions": Custom function is unsupported
#[test]
fn custom_function_is_unsupported() {
    let t = T::new();
    let e = t.err("SELECT ?x WHERE { ?x v:age ?a FILTER(<http://example.org/fn>(?a)) }");
    assert_unsupported(e, "http://example.org/fn");
}

// sparql-query "Literal canonicalisation is visible in queries": Integer lexical forms are equal
#[test]
fn integer_lexical_forms_are_equal() {
    let t = T::new();
    t.assert(&[(v("alice"), v("age"), int(1))]);
    assert!(t.ask("ASK { v:alice v:age \"01\"^^xsd:integer }"));
}

// sparql-query "Literal canonicalisation is visible in queries": sameTerm follows canonical encoding
#[test]
fn same_term_follows_canonical_encoding() {
    let t = T::new();
    assert!(t.ask("ASK { FILTER(sameTerm(\"01\"^^xsd:integer, 1)) }"));
}

// sparql-query "Literal canonicalisation is visible in queries": Date-time offsets are distinct terms
#[test]
fn datetime_offsets_are_distinct_terms() {
    let t = T::new();
    let (a, b) = (
        "\"2026-03-01T12:00:00+02:00\"^^xsd:dateTime",
        "\"2026-03-01T10:00:00Z\"^^xsd:dateTime",
    );
    assert!(!t.ask(&format!("ASK {{ FILTER(sameTerm({a}, {b})) }}")));
    assert!(t.ask(&format!("ASK {{ FILTER({a} = {b}) }}")));
}

// sparql-query "Literal canonicalisation is visible in queries": Pattern constant matches by term, FILTER by instant
#[test]
fn pattern_constant_matches_by_term_filter_by_instant() {
    let t = T::new();
    t.assert(&[(v("e"), v("at"), dt("2026-09-01T12:00:00Z"))]);
    assert!(!t.ask("ASK { v:e v:at \"2026-09-01T14:00:00+02:00\"^^xsd:dateTime }"));
    assert!(t.ask("ASK { v:e v:at ?t FILTER(?t = \"2026-09-01T14:00:00+02:00\"^^xsd:dateTime) }"));
    // and a shared variable joins by term
    t.assert(&[(v("f"), v("at"), dt("2026-09-01T14:00:00+02:00"))]);
    let got = t.col(
        "SELECT ?x ?y WHERE { ?x v:at ?t . ?y v:at ?t } ORDER BY ?x",
        "x",
    );
    assert_eq!(got, some(&[v("e"), v("f")]));
}

// sparql-query "Literal canonicalisation is visible in queries": Date-time keeps its offset in results
#[test]
fn datetime_keeps_its_offset_in_results() {
    let t = T::new();
    t.assert(&[(v("meeting"), v("at"), dt("2026-09-01T09:00:00+03:00"))]);
    let r =
        t.db.now()
            .sparql("SELECT ?t WHERE { v:meeting v:at ?t }")
            .unwrap();
    assert!(r
        .write_sparql_json()
        .unwrap()
        .contains("\"value\":\"2026-09-01T09:00:00+03:00\""));
    let s = r.solutions().unwrap();
    assert_eq!(s.get(0, "t"), Some(&dt("2026-09-01T09:00:00+03:00")));
}

// sparql-query "Literal canonicalisation is visible in queries": Ordering compares instants across offsets
#[test]
fn ordering_compares_instants_across_offsets() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("at"), dt("2026-09-01T12:00:00+02:00")),
        (v("b"), v("at"), dt("2026-09-01T11:00:00Z")),
    ]);
    assert_eq!(
        t.col("SELECT ?s WHERE { ?s v:at ?t } ORDER BY ?t", "s"),
        some(&[v("a"), v("b")])
    );
}

// sparql-query "Literal canonicalisation is visible in queries": TIMEZONE and TZ read the stored offset
#[test]
fn timezone_and_tz_read_the_stored_offset() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("at"), dt("2026-09-01T12:00:00+02:00")),
        (v("b"), v("at"), dt("2026-09-01T12:00:00")),
    ]);
    let r = t.sel("SELECT ?s (TZ(?t) AS ?z) (TIMEZONE(?t) AS ?d) WHERE { ?s v:at ?t } ORDER BY ?s");
    assert_eq!(r.get(0, "z"), Some(&s("+02:00")));
    assert_eq!(
        r.get(0, "d"),
        Some(&Value::Typed {
            lex: "PT2H".into(),
            datatype: xsd("dayTimeDuration")
        })
    );
    assert_eq!(r.get(1, "z"), Some(&s("")));
    assert_eq!(r.get(1, "d"), None);
}

// sparql-query "Literal canonicalisation is visible in queries": Language tag case
#[test]
fn language_tag_case() {
    let t = T::new();
    t.assert(&[(
        v("alice"),
        v("greeting"),
        Value::literal("Hallo", None, Some("DE")),
    )]);
    let got = t.col(
        "SELECT (LANG(?g) AS ?l) WHERE { v:alice v:greeting ?g FILTER(?g = \"Hallo\"@de) }",
        "l",
    );
    assert_eq!(got, some(&[s("de")]));
}

// sparql-query "Blank nodes and skolem IRIs in queries": Blank node label is not projected
#[test]
fn blank_node_label_is_not_projected() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("worksAt"), v("acme")),
        (v("acme"), v("locatedIn"), v("berlin")),
    ]);
    let r = t.sel("SELECT ?p WHERE { ?p v:worksAt _:c . _:c v:locatedIn v:berlin }");
    assert_eq!(r.vars, vec!["p".to_string()]);
    assert_eq!(r.rows, vec![vec![Some(v("alice"))]]);
}

// sparql-query "Blank nodes and skolem IRIs in queries": Anonymous node round trip
#[test]
fn anonymous_node_round_trip() {
    let t = T::new();
    let mut node = None;
    t.tx(|tx| {
        for _ in 0..6 {
            tx.new_node()?;
        }
        let n = tx.new_node()?;
        tx.assert(n, v("name"), s("anon"), Valid::ALWAYS)?;
        node = Some(n);
        Ok(())
    });
    let got = t.col("SELECT ?n WHERE { ?x v:name \"anon\" BIND(?x AS ?n) }", "n");
    assert_eq!(got, vec![Some(Value::Node(7))]);
    let back = t.col("SELECT ?n WHERE { <urn:tiramemsu:node:7> v:name ?n }", "n");
    assert_eq!(back, some(&[s("anon")]));
    let _ = node;
}

// sparql-query "Property paths before the path engine": Inverse single predicate
#[test]
fn inverse_single_predicate() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    assert_eq!(
        t.col("SELECT ?p WHERE { v:acme ^v:worksAt ?p }", "p"),
        some(&[v("alice")])
    );
}

// sparql-query "Parse errors report position": Syntax error location
#[test]
fn syntax_error_location() {
    let t = T::new();
    let (span, _) = assert_parse(t.err("SELECT ?s WHERE {\n  ?s v:p ?o\n  FILTER(\n}"));
    let span = span.expect("position");
    assert_eq!((span.line, span.column), (4, 1));
    assert_eq!(
        span.offset,
        "SELECT ?s WHERE {\n  ?s v:p ?o\n  FILTER(\n".len()
    );
}

// sparql-query "Parse errors report position": Undeclared prefix
#[test]
fn undeclared_prefix() {
    let t = T::new();
    let (_, msg) = assert_parse(t.err("SELECT ?s WHERE { ?s ex:p ?o }"));
    assert!(msg.contains("prefix") && msg.contains("not found"), "{msg}");
}

// sparql-query "Predeclared prefixes": Default vocabulary prefix
#[test]
fn default_vocabulary_prefix() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    let got = t.col(
        "SELECT ?c WHERE { <urn:tiramemsu:v:alice> v:worksAt ?c }",
        "c",
    );
    assert_eq!(got, some(&[v("acme")]));
}

// sparql-query "Predeclared prefixes": Database prefix table
#[test]
fn database_prefix_table() {
    let t = T::new();
    t.tx(|tx| {
        let node = tx.new_node()?;
        let sys = |n: &str| Value::iri(format!("{}{n}", vocab::SYS));
        tx.assert(sys("db"), sys("prefix"), node, Valid::ALWAYS)?;
        tx.assert(node, sys("prefixName"), s("schema"), Valid::ALWAYS)?;
        tx.assert(
            node,
            sys("prefixIri"),
            Value::iri("https://schema.org/"),
            Valid::ALWAYS,
        )?;
        tx.assert(
            v("p1"),
            Value::iri("https://schema.org/name"),
            s("Pat"),
            Valid::ALWAYS,
        )?;
        Ok(())
    });
    assert_eq!(
        t.col("SELECT ?n WHERE { ?p schema:name ?n }", "n"),
        some(&[s("Pat")])
    );
}

// sparql-query "Predeclared prefixes": Query prefix overrides
#[test]
fn query_prefix_overrides() {
    let t = T::new();
    t.assert(&[(Value::iri("http://example.org/x"), v("p"), int(1))]);
    let got = t.col(
        "PREFIX v: <http://example.org/> SELECT ?o WHERE { v:x <urn:tiramemsu:v:p> ?o }",
        "o",
    );
    assert_eq!(got, some(&[int(1)]));
}

// sparql-query "Predeclared prefixes": the configured @vocab is what v: means
#[test]
fn configured_vocab_is_the_v_prefix() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            Value::iri(format!("{}db", vocab::SYS)),
            Value::iri(format!("{}vocab", vocab::SYS)),
            Value::iri("http://voc.example/"),
            Valid::ALWAYS,
        )?;
        tx.assert(
            Value::iri("http://voc.example/a"),
            Value::iri("http://voc.example/p"),
            int(5),
            Valid::ALWAYS,
        )?;
        Ok(())
    });
    assert_eq!(
        t.col("SELECT ?o WHERE { v:a v:p ?o }", "o"),
        some(&[int(5)])
    );
}

// spargebra 0.4.7 parses `a - b - c` as `a - (b - c)`; the front end re-associates
// chains when the text has no parenthesised operand of that class
#[test]
fn arithmetic_chains_are_left_associative() {
    let t = T::new();
    let r = t.sel(
        "SELECT ((10 - 3 - 2) AS ?a) ((10 - 3 + 2) AS ?b) ((100 / 10 / 5) AS ?c) ((2 * 3 / 4) AS ?d) \
         ((10 - 3 - 2 - 1) AS ?e) WHERE { }",
    );
    assert_eq!(r.get(0, "a"), Some(&int(5)));
    assert_eq!(r.get(0, "b"), Some(&int(9)));
    assert_eq!(r.get(0, "c"), Some(&Value::Double(2.0)));
    assert_eq!(r.get(0, "d"), Some(&Value::Double(1.5)));
    assert_eq!(r.get(0, "e"), Some(&int(4)));
    // an explicitly parenthesised right operand keeps its grouping
    let r = t.sel("SELECT ((10 - (3 - 2)) AS ?a) ((100 / (10 / 5)) AS ?c) WHERE { }");
    assert_eq!(r.get(0, "a"), Some(&int(9)));
    assert_eq!(r.get(0, "c"), Some(&Value::Double(50.0)));
}
