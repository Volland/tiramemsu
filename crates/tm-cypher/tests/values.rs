//! Value model: equality, ordering, grouping keys, three-valued logic, JSON.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use tm_core::{Eid, Value};
use tm_cypher::value::{cmp3, eq3, group_key, order, Val};
use tm_cypher::{CypherResult, CypherValue, NodeValue, RelValue};

fn n(x: i64) -> Val {
    Val::Int(x)
}

// cypher-read "ORDER BY, SKIP and LIMIT": the cross-type order, ascending
#[test]
fn cross_type_order() {
    let mut m = BTreeMap::new();
    m.insert("a".to_string(), n(1));
    let vals = vec![
        Val::Null,
        Val::Float(f64::NAN),
        n(3),
        Val::Bool(true),
        Val::Bool(false),
        Val::Str("b".into()),
        Val::Date(10),
        Val::LocalDateTime(5),
        Val::DateTime { ms: 5, tz: 0 },
        Val::Path(vec![]),
        Val::List(vec![n(1)]),
        Val::Rel(Eid::new(1)),
        Val::Node(Value::iri("urn:tiramemsu:v:a")),
        Val::Map(m),
    ];
    let mut sorted = vals.clone();
    sorted.sort_by(order);
    let names: Vec<&str> = sorted.iter().map(Val::type_name).collect();
    assert_eq!(
        names,
        [
            "Map",
            "Node",
            "Relationship",
            "List",
            "Path",
            "DateTime",
            "LocalDateTime",
            "Date",
            "String",
            "Boolean",
            "Boolean",
            "Integer",
            "Float",
            "Null"
        ]
    );
    // false before true, NaN after numbers
    assert_eq!(order(&Val::Bool(false), &Val::Bool(true)), Ordering::Less);
    assert_eq!(
        order(&Val::Float(f64::NAN), &n(i64::MAX)),
        Ordering::Greater
    );
}

// numbers compare numerically across integer and float; datetimes by instant
#[test]
fn numeric_and_temporal_comparison() {
    assert_eq!(cmp3(&n(10), &Val::Float(9.5)), Some(Ordering::Greater));
    assert_eq!(eq3(&n(30), &Val::Float(30.0)), Some(true));
    assert_eq!(
        eq3(&Val::Float(f64::NAN), &Val::Float(f64::NAN)),
        Some(false)
    );
    let a = Val::DateTime { ms: 1_000, tz: 120 };
    let b = Val::DateTime { ms: 1_000, tz: 0 };
    assert_eq!(eq3(&a, &b), Some(true));
    assert_eq!(cmp3(&a, &Val::Str("x".into())), None);
    assert_eq!(eq3(&Val::Null, &Val::Null), None);
    assert_eq!(eq3(&n(1), &Val::Str("1".into())), Some(false));
}

// lists: null inside makes the comparison unknown
#[test]
fn list_equality_with_null() {
    let a = Val::List(vec![n(1), Val::Null]);
    let b = Val::List(vec![n(1), n(2)]);
    assert_eq!(eq3(&a, &b), None);
    assert_eq!(eq3(&a, &Val::List(vec![n(2), Val::Null])), Some(false));
}

// the dual view: a statement in node form equals its relationship
#[test]
fn statement_forms_are_equal() {
    let e = Eid::new(7);
    assert_eq!(eq3(&Val::Rel(e), &Val::Node(Value::Stmt(e))), Some(true));
    assert_eq!(eq3(&Val::Rel(e), &Val::Rel(Eid::new(8))), Some(false));
}

// DISTINCT / grouping equivalence: nulls equal, 1 and 1.0 equal
#[test]
fn group_keys() {
    assert_eq!(group_key(&Val::Null), group_key(&Val::Null));
    assert_eq!(group_key(&n(1)), group_key(&Val::Float(1.0)));
    assert_ne!(group_key(&n(1)), group_key(&Val::Str("1".into())));
    assert_ne!(
        group_key(&Val::List(vec![n(1)])),
        group_key(&Val::List(vec![n(2)]))
    );
}

// JSON encoding of every kind (design Decision 10)
#[test]
fn json_encoding() {
    let node = NodeValue {
        term: Value::iri("urn:tiramemsu:v:alice"),
        element_id: "urn:tiramemsu:v:alice".into(),
        labels: vec!["Person".into()],
        properties: BTreeMap::from([("name".to_string(), CypherValue::String("Alice".into()))]),
    };
    let rel = RelValue {
        eid: Eid::new(1),
        element_id: "urn:tiramemsu:stmt:1".into(),
        rel_type: "worksAt".into(),
        start_element_id: "urn:tiramemsu:v:alice".into(),
        end_element_id: "urn:tiramemsu:v:acme".into(),
        properties: BTreeMap::new(),
    };
    let res = CypherResult {
        columns: vec![
            "a".into(),
            "r".into(),
            "d".into(),
            "t".into(),
            "l".into(),
            "big".into(),
            "f".into(),
        ],
        rows: vec![vec![
            CypherValue::Node(Box::new(node)),
            CypherValue::Relationship(Box::new(rel)),
            CypherValue::Date(20_148),
            CypherValue::DateTime {
                ms: 1_772_353_200_000,
                tz: 120,
            },
            CypherValue::LocalDateTime(1_772_353_200_000),
            CypherValue::Integer(9_007_199_254_740_993),
            CypherValue::List(vec![
                CypherValue::Float(1.5),
                CypherValue::Null,
                CypherValue::Boolean(true),
            ]),
        ]],
        report: None,
    };
    insta::assert_json_snapshot!(res.to_json());
}
