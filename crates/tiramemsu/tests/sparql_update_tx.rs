//! `sparql-update`: transactions, the report, unsupported operations and parse errors.
mod sparql_common;
use sparql_common::*;
use tiramemsu::*;

// sparql-update "Updates run as one transaction on the current view": Multi-operation request is one transaction
#[test]
fn multi_operation_request_is_one_transaction() {
    let t = T::new();
    let before = t.last_t();
    let r =
        t.upd("INSERT DATA { v:alice v:worksAt v:acme } ; INSERT DATA { v:bob v:worksAt v:acme }");
    assert_eq!(t.last_t(), before + 1);
    assert_eq!(r.t.0, before + 1);
    assert_eq!(r.asserted.len(), 2);
    let adds: Vec<_> =
        t.db.now()
            .triples(None, None, None)
            .unwrap()
            .iter()
            .map(|x| x.t_add)
            .collect();
    assert_eq!(adds, vec![r.t, r.t]);
}

// sparql-update "Updates run as one transaction on the current view": Later operation sees earlier one
#[test]
fn later_operation_sees_earlier_one() {
    let t = T::new();
    t.upd("INSERT DATA { v:alice v:age 41 } ; INSERT { ?p v:adult true } WHERE { ?p v:age ?a FILTER(?a >= 18) }");
    assert!(t.has(&v("alice"), &v("adult"), &Value::Bool(true)));
}

// sparql-update "Updates run as one transaction on the current view": Failure leaves no trace
#[test]
fn failure_leaves_no_trace() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            v("email"),
            Value::iri(vocab::SYS_UNIQUE),
            Value::Bool(true),
            Valid::ALWAYS,
        )?;
        tx.assert(v("alice"), v("email"), s("a@x.org"), Valid::ALWAYS)?;
        Ok(())
    });
    let (last, before) = (t.last_t(), t.live());
    let e = t.err("INSERT DATA { v:x v:p 1 } ; INSERT DATA { v:bob v:email \"a@x.org\" }");
    assert!(matches!(e, Error::UniqueViolation { .. }), "{e:?}");
    assert_eq!(t.last_t(), last);
    assert_eq!(t.live(), before);
    assert!(!t.has(&v("x"), &v("p"), &int(1)));
}

// sparql-update "Updates run as one transaction on the current view": Update on a historical view is rejected
#[test]
fn update_on_a_historical_view_is_rejected() {
    let t = T::new();
    t.assert(&[(v("a"), v("b"), v("c"))]);
    for view in [
        t.db.as_of(TimeRef::Tx(1)),
        t.db.history(),
        t.db.now().valid_at(5),
    ] {
        let e = view.sparql("INSERT DATA { v:a v:b v:d }").unwrap_err();
        assert_unsupported(e, "update on a non-current view");
    }
    assert!(!t.has(&v("a"), &v("b"), &v("d")));
    // a speculative view is not the plain current view either
    let e =
        t.db.with(
            |_| Ok(()),
            |view| view.sparql("INSERT DATA { v:a v:b v:d }"),
        )
        .unwrap_err();
    assert_unsupported(e, "update on a non-current view");
}

// sparql-update "Update report": Report lists new and existing eids
#[test]
fn report_lists_new_and_existing_eids() {
    let t = T::new();
    let e1 = t.assert(&[(v("alice"), v("worksAt"), v("acme"))]).asserted[0];
    let r = t.upd("INSERT DATA { v:alice v:worksAt v:acme . v:alice v:age 41 }");
    assert_eq!(r.existing, vec![e1]);
    assert_eq!(r.asserted.len(), 1);
    assert!(r.retracted.is_empty());
}

// sparql-update "Update report": No-op request still commits
#[test]
fn no_op_request_still_commits() {
    let t = T::new();
    let before = t.last_t();
    let r = t.upd("DELETE DATA { v:nobody v:knows v:noone }");
    assert_eq!(r.t.0, before + 1);
    assert!(r.asserted.is_empty() && r.existing.is_empty() && r.retracted.is_empty());
}

// sparql-update "Unsupported graph operations": CLEAR DEFAULT is rejected
#[test]
fn clear_default_is_rejected() {
    let t = T::new();
    t.assert(&[(v("a"), v("b"), v("c"))]);
    assert_unsupported(t.err("CLEAR DEFAULT"), "CLEAR DEFAULT");
    assert_unsupported(t.err("CLEAR ALL"), "CLEAR ALL");
    assert!(t.has(&v("a"), &v("b"), &v("c")));
}

// sparql-update "Unsupported graph operations": DROP SILENT ALL is still rejected
#[test]
fn drop_silent_all_is_still_rejected() {
    let t = T::new();
    assert_unsupported(t.err("DROP SILENT ALL"), "DROP ALL");
    assert_unsupported(t.err("DROP DEFAULT"), "DROP DEFAULT");
}

// sparql-update "Unsupported update operations": Rejected operation aborts the whole request
#[test]
fn rejected_operation_aborts_the_whole_request() {
    let t = T::new();
    let before = t.last_t();
    assert_unsupported(
        t.err("INSERT DATA { v:a v:b v:c } ; LOAD <http://example.org/data.ttl>"),
        "LOAD",
    );
    assert!(!t.has(&v("a"), &v("b"), &v("c")));
    assert_eq!(t.last_t(), before);
}

// sparql-update "Unsupported graph operations": ADD, MOVE, COPY and LOAD name themselves
#[test]
fn add_move_copy_are_rejected() {
    let t = T::new();
    let before = t.last_t();
    for (q, want) in [
        (
            "ADD <http://example.org/a> TO <http://example.org/b>",
            "ADD",
        ),
        (
            "MOVE <http://example.org/a> TO <http://example.org/b>",
            "MOVE",
        ),
        (
            "COPY <http://example.org/a> TO <http://example.org/b>",
            "COPY",
        ),
        (
            "COPY SILENT <http://example.org/a> TO <http://example.org/b>",
            "COPY",
        ),
        ("LOAD <http://example.org/a>", "LOAD"),
    ] {
        assert_unsupported(t.err(q), want);
    }
    assert_eq!(t.last_t(), before);
}

// sparql-update "Time- and graph-scoped WHERE in updates": a time IRI as the graph of INSERT DATA is rejected
#[test]
fn time_iri_as_insert_data_graph_is_rejected() {
    let t = T::new();
    let before = t.last_t();
    let (_, msg) =
        assert_parse(t.err("INSERT DATA { GRAPH <urn:tiramemsu:tm:asOf/150> { v:a v:b v:c } }"));
    assert!(msg.contains("SERVICE"), "{msg}");
    assert_eq!(t.last_t(), before);
}

// sparql-update "Update parse errors report position": Variable in INSERT DATA
#[test]
fn variable_in_insert_data() {
    let t = T::new();
    let (span, _) = assert_parse(t.err("INSERT DATA { ?s v:p 1 }"));
    assert!(span.is_some());
}

// sparql-update "Update parse errors report position": Blank node in DELETE DATA
#[test]
fn blank_node_in_delete_data() {
    let t = T::new();
    assert_parse(t.err("DELETE DATA { _:b v:p 1 }"));
    assert_parse(t.err("DELETE WHERE { _:b v:p ?o }"));
    assert_parse(t.err("DELETE DATA { ?s v:p 1 }"));
}
