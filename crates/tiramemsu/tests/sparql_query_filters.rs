//! `sparql-query`: FILTER and expression errors, aggregates, subqueries and
//! solution modifiers.
mod sparql_common;
use sparql_common::*;
use tiramemsu::Value;

fn ages_alice_bob_unknown() -> T {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("age"), int(41)),
        (v("bob"), v("age"), s("unknown")),
    ]);
    t
}

// sparql-query "FILTER and expression errors": Type error removes the row
#[test]
fn type_error_removes_the_row() {
    let t = ages_alice_bob_unknown();
    assert_eq!(
        t.col("SELECT ?p WHERE { ?p v:age ?a FILTER(?a > 30) }", "p"),
        some(&[v("alice")])
    );
}

// sparql-query "FILTER and expression errors": Negating an error still removes the row
#[test]
fn negating_an_error_still_removes_the_row() {
    let t = ages_alice_bob_unknown();
    assert!(t
        .sel("SELECT ?p WHERE { ?p v:age ?a FILTER(!(?a > 30)) }")
        .rows
        .is_empty());
}

// sparql-query "FILTER and expression errors": Unbound variable in filter
#[test]
fn unbound_variable_in_filter() {
    let t = T::new();
    t.assert(&[(v("alice"), v("name"), s("Alice"))]);
    let q = "SELECT ?p WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } FILTER(?a < 100) }";
    assert!(t.sel(q).rows.is_empty());
}

// sparql-query "FILTER and expression errors": Error rescued by OR
#[test]
fn error_rescued_by_or() {
    let t = ages_alice_bob_unknown();
    let got = t.col(
        "SELECT ?p WHERE { ?p v:age ?a FILTER(?a > 30 || true) } ORDER BY ?p",
        "p",
    );
    assert_eq!(got, some(&[v("alice"), v("bob")]));
}

// sparql-query "FILTER and expression errors": Numeric equality across datatypes
#[test]
fn numeric_equality_across_datatypes() {
    let t = T::new();
    t.assert(&[(v("alice"), v("age"), int(41))]);
    assert!(t.ask("ASK { v:alice v:age ?a FILTER(?a = 41.0) }"));
}

fn worksat() -> T {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("worksAt"), v("acme")),
        (v("bob"), v("worksAt"), v("acme")),
        (v("carol"), v("worksAt"), v("initech")),
    ]);
    t
}

// sparql-query "Aggregates and grouping": Count per group
#[test]
fn count_per_group() {
    let t = worksat();
    let r = t.sel("SELECT ?c (COUNT(?p) AS ?n) WHERE { ?p v:worksAt ?c } GROUP BY ?c ORDER BY ?c");
    assert_eq!(
        r.rows,
        vec![
            vec![Some(v("acme")), Some(int(2))],
            vec![Some(v("initech")), Some(int(1))]
        ]
    );
}

// sparql-query "Aggregates and grouping": HAVING filters groups
#[test]
fn having_filters_groups() {
    let t = worksat();
    let r = t.sel(
        "SELECT ?c (COUNT(?p) AS ?n) WHERE { ?p v:worksAt ?c } GROUP BY ?c HAVING (COUNT(?p) > 1) ORDER BY ?c",
    );
    assert_eq!(r.rows, vec![vec![Some(v("acme")), Some(int(2))]]);
}

// sparql-query "Aggregates and grouping": Count over no solutions
#[test]
fn count_over_no_solutions() {
    let t = T::new();
    let r = t.sel("SELECT (COUNT(*) AS ?n) WHERE { ?p v:neverUsed ?o }");
    assert_eq!(r.rows, vec![vec![Some(int(0))]]);
}

// sparql-query "Aggregates and grouping": GROUP_CONCAT with separator
#[test]
fn group_concat_with_separator() {
    let t = T::new();
    let r = t.sel(
        "SELECT (GROUP_CONCAT(?n; SEPARATOR=\"|\") AS ?all) WHERE { VALUES ?n { \"a\" \"b\" } }",
    );
    let all = r.get(0, "all").cloned().unwrap();
    assert!(all == s("a|b") || all == s("b|a"), "{all:?}");
}

#[test]
fn other_aggregates() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("n"), int(1)),
        (v("b"), v("n"), int(2)),
        (v("c"), v("n"), int(2)),
    ]);
    let r = t.sel(
        "SELECT (SUM(?x) AS ?s) (MIN(?x) AS ?lo) (MAX(?x) AS ?hi) (COUNT(DISTINCT ?x) AS ?d) (AVG(?x) AS ?m) \
         WHERE { ?p v:n ?x }",
    );
    assert_eq!(r.get(0, "s"), Some(&int(5)));
    assert_eq!(r.get(0, "lo"), Some(&int(1)));
    assert_eq!(r.get(0, "hi"), Some(&int(2)));
    assert_eq!(r.get(0, "d"), Some(&int(2)));
    assert!(matches!(r.get(0, "m"), Some(Value::Double(x)) if (x - 5.0 / 3.0).abs() < 1e-9));
}

// sparql-query "Subqueries": Top-N per subquery
#[test]
fn top_n_per_subquery() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("age"), int(41)),
        (v("bob"), v("age"), int(30)),
        (v("carol"), v("age"), int(25)),
        (v("alice"), v("name"), s("Alice")),
    ]);
    let q = "SELECT ?p ?n WHERE { { SELECT ?p WHERE { ?p v:age ?a } ORDER BY DESC(?a) LIMIT 2 } \
             OPTIONAL { ?p v:name ?n } } ORDER BY ?p";
    let r = t.sel(q);
    assert_eq!(
        r.rows,
        vec![
            vec![Some(v("alice")), Some(s("Alice"))],
            vec![Some(v("bob")), None]
        ]
    );
}

// sparql-query "Subqueries": Inner variables are hidden
#[test]
fn inner_variables_are_hidden() {
    let t = T::new();
    t.assert(&[(v("alice"), v("age"), int(41))]);
    let r = t.sel("SELECT ?a WHERE { { SELECT ?p WHERE { ?p v:age ?a } } }");
    assert_eq!(r.rows, vec![vec![None]]);
}

// sparql-query "Solution modifiers": Numeric order is by value
#[test]
fn numeric_order_is_by_value() {
    let t = T::new();
    t.assert(&[(v("a"), v("rank"), int(9)), (v("b"), v("rank"), int(10))]);
    assert_eq!(
        t.col("SELECT ?s WHERE { ?s v:rank ?r } ORDER BY ?r", "s"),
        some(&[v("a"), v("b")])
    );
}

// sparql-query "Solution modifiers": Descending string order with limit and offset
#[test]
fn descending_string_order_with_limit_and_offset() {
    let t = T::new();
    t.assert(&[
        (v("p1"), v("name"), s("ann")),
        (v("p2"), v("name"), s("bob")),
        (v("p3"), v("name"), s("cy")),
        (v("p4"), v("name"), s("dee")),
    ]);
    let got = t.col(
        "SELECT ?n WHERE { ?p v:name ?n } ORDER BY DESC(?n) LIMIT 2 OFFSET 1",
        "n",
    );
    assert_eq!(got, some(&[s("cy"), s("bob")]));
}

// sparql-query "Solution modifiers": Unbound sorts first
#[test]
fn unbound_sorts_first() {
    let t = T::new();
    t.assert(&[
        (v("alice"), v("name"), s("A")),
        (v("alice"), v("age"), int(41)),
        (v("bob"), v("name"), s("B")),
    ]);
    let r = t.sel("SELECT ?p ?a WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } } ORDER BY ?a");
    assert_eq!(r.rows[0], vec![Some(v("bob")), None]);
    assert_eq!(r.rows[1], vec![Some(v("alice")), Some(int(41))]);
}

// sparql-query "Solution modifiers": DISTINCT with a non-projected sort key
#[test]
fn distinct_with_non_projected_key_is_unsupported() {
    let t = T::new();
    let e = t.err("SELECT DISTINCT ?s WHERE { ?s v:knows ?o } ORDER BY ?o");
    assert_unsupported(e, "ORDER BY non-projected variable with DISTINCT");
}

#[test]
fn distinct_order_limit_slices_after_distinct() {
    let t = T::new();
    t.assert(&[
        (v("a"), v("knows"), v("x")),
        (v("a"), v("knows"), v("y")),
        (v("b"), v("knows"), v("x")),
        (v("c"), v("knows"), v("x")),
    ]);
    let got = t.col(
        "SELECT DISTINCT ?s WHERE { ?s v:knows ?o } ORDER BY ?s LIMIT 2",
        "s",
    );
    assert_eq!(got, some(&[v("a"), v("b")]));
}
