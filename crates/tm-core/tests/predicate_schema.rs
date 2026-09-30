//! Spec `predicate-schema`: flags as versioned statements, validation, cardinality
//! one, unique with upsert, value types, the edge flag and schema-change checks.

mod common;
use common::*;

use tm_core::vocab::*;
use tm_core::*;

fn flag(db: &mut TestDb, p: &str, f: &str, o: Value) -> Eid {
    let mut e = None;
    db.tx(|tx| {
        e = Some(tx.assert(iri(p), sys(f), o, Valid::ALWAYS)?.eid());
        Ok(())
    });
    e.unwrap()
}

fn put(db: &mut TestDb, s: &str, p: &str, o: Value, valid: Valid) -> Eid {
    let mut e = None;
    db.tx(|tx| {
        e = Some(tx.assert(iri(s), iri(p), o, valid)?.eid());
        Ok(())
    });
    e.unwrap()
}

host_test! {
    /// Schema flags are versioned statements.
    fn flags_are_versioned_statements(db) {
        let a = put(db, "alice", "likes", iri("tea"), Valid::ALWAYS);
        let b = put(db, "alice", "likes", iri("coffee"), Valid::ALWAYS);
        assert!(db.is_live(a) && db.is_live(b));
        while db.last_t() < 9 {
            db.tx(|_| Ok(()));
        }
        let f = flag(db, "age", "cardinality", sys("one"));
        assert_eq!(db.row(f).t_add, TxId(10));
        assert!(db.as_of(9).iter().all(|t| t.eid != f));
        assert!(db.is_live(f));
        // takes effect within its transaction
        let r = db.try_tx(|tx| {
            tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
            tx.assert(iri("alice"), iri("email"), lit("a@x.org"), Valid::ALWAYS)?;
            tx.assert(iri("bob"), iri("email"), lit("a@x.org"), Valid::ALWAYS)?;
            Ok(())
        });
        assert_err!(r, Error::UniqueViolation { .. });
        // retracting a flag lifts the constraint
        let u = flag(db, "email", "unique", Value::Bool(true));
        put(db, "alice", "email", lit("a@x.org"), Valid::ALWAYS);
        db.tx(|tx| tx.retract(u).map(|_| ()));
        put(db, "bob", "email", lit("a@x.org"), Valid::ALWAYS);
    }
}

host_test! {
    /// Schema flag values are validated.
    fn flag_values_are_validated(db) {
        flag(db, "height", "cardinality", sys("many"));
        let cp = db.id(&sys("cardinality"));
        let r = db.try_tx(|tx| tx.assert(iri("age"), sys("cardinality"), lit("one"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { p, got: Tag::ShortStr, .. } if p == cp);
        let r = db.try_tx(|tx| tx.assert(sys("reason"), sys("cardinality"), sys("one"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ReservedNamespace(_));
        let r = db.try_tx(|tx| tx.assert(iri("x"), sys("unique"), lit("yes"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { .. });
        let r = db.try_tx(|tx| tx.assert(iri("x"), sys("valueType"), Value::Int(1), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { .. });
        let r = db.try_tx(|tx| tx.assert(iri("x"), sys("valueType"), sys("NOPE"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { .. });
        let r = db.try_tx(|tx| {
            let n = tx.new_node()?;
            tx.assert(n, sys("isEdge"), Value::Bool(true), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::InvalidTerm { position: Position::Subject, .. });
        // changing a flag value retracts the previous one with kind cardinality
        let one = flag(db, "tag", "cardinality", sys("one"));
        let many = flag(db, "tag", "cardinality", sys("many"));
        assert_eq!(db.row(one).ret_kind, Some(RetKind::Cardinality));
        assert!(db.is_live(many));
        let card = db.id(&sys("cardinality"));
        let tag = db.id(&iri("tag"));
        assert_eq!(db.now(Some(tag), Some(card), None).len(), 1);
        // unique set to false
        flag(db, "email", "unique", Value::Bool(false));
        put(db, "alice", "email", lit("a@x.org"), Valid::ALWAYS);
        put(db, "bob", "email", lit("a@x.org"), Valid::ALWAYS);
    }
}

// @lat: [[tests#Operations#Cardinality One Does Not Replay]]
host_test! {
    fn cardinality_one_replaces_without_carrying_annotations(db) {
        flag(db, "age", "cardinality", sys("one"));
        let (mut e1, mut ann) = (None, None);
        db.tx(|tx| {
            let e = tx.assert(iri("alice"), iri("age"), Value::Int(30), Valid::ALWAYS)?.eid();
            ann = Some(tx.assert(e, iri("source"), iri("form"), Valid::ALWAYS)?.eid());
            e1 = Some(e);
            Ok(())
        });
        let new = put(db, "alice", "age", Value::Int(31), Valid::ALWAYS);
        assert_eq!(db.row(e1.unwrap()).ret_kind, Some(RetKind::Cardinality));
        assert_eq!(db.row(ann.unwrap()).ret_kind, Some(RetKind::Cardinality));
        assert!(db.now(Some(new.oid()), None, None).is_empty());
        assert!(db.is_live(new));
    }
}

host_test! {
    /// Cardinality one replaces overlapping objects — the other scenarios.
    fn cardinality_one_scenarios(db) {
        flag(db, "worksAt", "cardinality", sys("one"));
        flag(db, "age", "cardinality", sys("one"));
        let a = put(db, "alice", "worksAt", iri("acme"), Valid::between(day("2020-01-01"), day("2022-01-01")));
        let g = put(db, "alice", "worksAt", iri("globex"), Valid::from(day("2022-01-01")));
        assert!(db.is_live(a) && db.is_live(g));
        let b = put(db, "bob", "worksAt", iri("acme"), Valid::between(day("2020-01-01"), day("2025-01-01")));
        let bg = put(db, "bob", "worksAt", iri("globex"), Valid::from(day("2024-01-01")));
        let row = db.row(b);
        assert_eq!(row.ret_kind, Some(RetKind::Cardinality));
        assert_eq!(row.v_to, Some(day("2025-01-01")));
        assert!(db.is_live(bg));
        // re-asserting the same value retracts nothing
        put(db, "carol", "age", Value::Int(30), Valid::ALWAYS);
        let rep = db.tx(|tx| {
            let r = tx.assert(iri("carol"), iri("age"), Value::Int(30), Valid::ALWAYS)?;
            assert!(!r.is_new());
            Ok(())
        });
        assert!(rep.retracted.is_empty());
        // create on a cardinality-one predicate
        let first = put(db, "dave", "age", Value::Int(30), Valid::ALWAYS);
        let mut second = None;
        let mut third = None;
        db.tx(|tx| {
            second = Some(tx.create(iri("dave"), iri("age"), Value::Int(30), Valid::ALWAYS)?);
            Ok(())
        });
        assert!(db.is_live(first) && db.is_live(second.unwrap()));
        let rep = db.tx(|tx| {
            third = Some(tx.create(iri("dave"), iri("age"), Value::Int(31), Valid::ALWAYS)?);
            Ok(())
        });
        assert_eq!(
            rep.retracted,
            vec![(first, RetKind::Cardinality), (second.unwrap(), RetKind::Cardinality)]
        );
        let dave = db.id(&iri("dave"));
        let age = db.id(&iri("age"));
        assert_eq!(db.now(Some(dave), Some(age), None).len(), 1);
        // other subjects are untouched
        let bob40 = put(db, "bob", "age", Value::Int(40), Valid::ALWAYS);
        put(db, "alice", "age", Value::Int(31), Valid::ALWAYS);
        assert!(db.is_live(bob40));
    }
}

// @lat: [[tests#Operations#Unique Rejects Second Subject]]
host_test! {
    fn unique_rejects_second_subject(db) {
        flag(db, "email", "unique", Value::Bool(true));
        put(db, "alice", "email", lit("a@x.org"), Valid::ALWAYS);
        let before = db.snapshot();
        let r = db.try_tx(|tx| tx.assert(iri("bob"), iri("email"), lit("a@x.org"), Valid::ALWAYS).map(|_| ()));
        let (email, alice) = (db.id(&iri("email")), db.id(&iri("alice")));
        let o = db.id(&lit("a@x.org"));
        assert_err!(r, Error::UniqueViolation { p, o: oo, existing } if p == email && oo == o && existing == alice);
        assert_eq!(db.snapshot(), before);
        assert!(db.oid(&iri("bob")).is_none());
    }
}

host_test! {
    /// Unique predicates allow one live subject per value — other scenarios.
    fn unique_scenarios(db) {
        flag(db, "email", "unique", Value::Bool(true));
        let a = put(db, "alice", "email", lit("a@x.org"), Valid::between(day("2020-01-01"), day("2021-01-01")));
        let r = db.try_tx(|tx| {
            tx.assert(iri("bob"), iri("email"), lit("a@x.org"), Valid::from(day("2022-01-01"))).map(|_| ())
        });
        assert_err!(r, Error::UniqueViolation { .. });
        // same subject, new episode
        let a2 = put(db, "alice", "email", lit("a@x.org"), Valid::from(day("2022-01-01")));
        assert_ne!(a, a2);
        assert!(db.is_live(a) && db.is_live(a2));
        // value released by retraction
        let alice = db.id(&iri("alice"));
        db.tx(|tx| tx.retract_matching(Some(alice), None, None).map(|_| ()));
        put(db, "bob", "email", lit("a@x.org"), Valid::ALWAYS);
    }
}

// @lat: [[tests#Operations#Upsert Returns Existing Node]]
host_test! {
    fn upsert_returns_existing_or_new_node(db) {
        flag(db, "email", "unique", Value::Bool(true));
        put(db, "alice", "email", lit("a@x.org"), Valid::ALWAYS);
        let alice = db.id(&iri("alice"));
        let mut got = None;
        let rep = db.tx(|tx| {
            got = Some(tx.upsert(iri("email"), lit("a@x.org"))?);
            Ok(())
        });
        assert_eq!(got, Some(alice));
        assert!(rep.asserted.is_empty());
        let rep = db.tx(|tx| {
            got = Some(tx.upsert(iri("email"), lit("new@x.org"))?);
            Ok(())
        });
        let n = got.unwrap();
        assert_eq!(n.tag().unwrap(), Tag::Node);
        let email = db.id(&iri("email"));
        let row = db.row(rep.asserted[0]);
        assert_eq!((row.s, row.p, row.o), (n, email, db.id(&lit("new@x.org"))));
        assert!(db.is_live(rep.asserted[0]));
    }
}

host_test! {
    /// Upsert on a unique predicate — twice in one transaction, non-unique predicate.
    fn upsert_twice_and_not_unique(db) {
        flag(db, "email", "unique", Value::Bool(true));
        let rep = db.tx(|tx| {
            let a = tx.upsert(iri("email"), lit("new@x.org"))?;
            let b = tx.upsert(iri("email"), lit("new@x.org"))?;
            assert_eq!(a, b);
            Ok(())
        });
        assert_eq!(rep.asserted.len(), 1);
        let r = db.try_tx(|tx| tx.upsert(iri("name"), lit("x")).map(|_| ()));
        assert_err!(r, Error::NotUniquePredicate(_));
    }
}

host_test! {
    /// Value type constrains objects.
    fn value_type_constrains_objects(db) {
        flag(db, "age", "valueType", Value::iri(XSD_INTEGER));
        let r = db.try_tx(|tx| tx.assert(iri("alice"), iri("age"), lit("thirty"), Valid::ALWAYS).map(|_| ()));
        let age = db.id(&iri("age"));
        let xint = db.id(&Value::iri(XSD_INTEGER));
        assert_err!(r, Error::ValueTypeMismatch { p, expected, got: Tag::ShortStr } if p == age && expected == xint);
        flag(db, "big", "valueType", Value::iri(XSD_INTEGER));
        put(db, "alice", "big", Value::big_integer("1180591620717411303424"), Valid::ALWAYS);
        flag(db, "name", "valueType", Value::iri(XSD_STRING));
        put(db, "alice", "name", lit("Al"), Valid::ALWAYS);
        put(db, "alice", "name", lit("Alexandrina"), Valid::ALWAYS);
        flag(db, "supportedBy", "valueType", sys("STMT"));
        let r = db.try_tx(|tx| {
            let n = tx.new_node()?;
            tx.assert(iri("b"), iri("supportedBy"), n, Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::ValueTypeMismatch { got: Tag::Node, .. });
        // other datatypes match TYPED values with that datatype
        flag(db, "shape", "valueType", Value::iri("urn:dt:wkt"));
        put(db, "x", "shape", Value::literal("POINT(1 2)", Some("urn:dt:wkt"), None), Valid::ALWAYS);
        let r = db.try_tx(|tx| {
            tx.assert(iri("x"), iri("shape"), Value::literal("P", Some("urn:dt:other"), None), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::ValueTypeMismatch { got: Tag::Typed, .. });
        // create is checked too
        let r = db.try_tx(|tx| tx.create(iri("bob"), iri("age"), lit("old"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { .. });
    }
}

host_test! {
    /// Edge flag is stored.
    fn edge_flag_is_stored(db) {
        let f = flag(db, "nickname", "isEdge", Value::Bool(true));
        put(db, "alice", "nickname", lit("Al"), Valid::ALWAYS);
        assert!(db.is_live(f));
        let mut schema = None;
        db.tx(|tx| {
            let p = tx.encode(iri("nickname"))?;
            schema = Some(tx.schema(p)?);
            Ok(())
        });
        assert_eq!(schema.unwrap().is_edge, Some(true));
    }
}

host_test! {
    /// Schema changes violated by live data are rejected.
    fn schema_changes_violated_by_live_data(db) {
        let a30 = put(db, "alice", "age", Value::Int(30), Valid::ALWAYS);
        let a31 = put(db, "alice", "age", Value::Int(31), Valid::from(5));
        let before = db.snapshot();
        let r = db.try_tx(|tx| tx.assert(iri("age"), sys("cardinality"), sys("one"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SchemaConflict { violating } if violating == vec![a30, a31]);
        assert_eq!(db.snapshot(), before);
        // disjoint episodes do not conflict
        put(db, "alice", "worksAt", iri("acme"), Valid::between(day("2020-01-01"), day("2022-01-01")));
        put(db, "alice", "worksAt", iri("globex"), Valid::from(day("2022-01-01")));
        flag(db, "worksAt", "cardinality", sys("one"));
        // unique over duplicate values
        let e1 = put(db, "alice", "email", lit("a@x.org"), Valid::ALWAYS);
        let e2 = put(db, "bob", "email", lit("a@x.org"), Valid::ALWAYS);
        put(db, "carol", "email", lit("c@x.org"), Valid::ALWAYS);
        let r = db.try_tx(|tx| tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SchemaConflict { violating } if violating == vec![e1, e2]);
        // value type over mismatching data
        let b = put(db, "bob", "age", lit("old"), Valid::ALWAYS);
        let r = db.try_tx(|tx| tx.assert(iri("age"), sys("valueType"), Value::iri(XSD_INTEGER), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SchemaConflict { violating } if violating == vec![b]);
        // conflict created earlier in the same transaction
        let r = db.try_tx(|tx| {
            tx.assert(iri("dan"), iri("mail"), lit("d@x.org"), Valid::ALWAYS)?;
            tx.assert(iri("eve"), iri("mail"), lit("d@x.org"), Valid::ALWAYS)?;
            tx.assert(iri("mail"), sys("unique"), Value::Bool(true), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::SchemaConflict { .. });
        // many / false / isEdge / retraction never conflict
        flag(db, "age", "cardinality", sys("many"));
        flag(db, "email", "unique", Value::Bool(false));
        flag(db, "age", "isEdge", Value::Bool(false));
    }
}

host_test! {
    /// Order of schema checks.
    fn order_of_schema_checks(db) {
        db.tx(|tx| {
            tx.assert(iri("age"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
            tx.assert(iri("age"), sys("valueType"), Value::iri(XSD_INTEGER), Valid::ALWAYS)?;
            tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
            tx.assert(iri("email"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
            Ok(())
        });
        let a30 = put(db, "alice", "age", Value::Int(30), Valid::ALWAYS);
        let r = db.try_tx(|tx| tx.assert(iri("alice"), iri("age"), lit("thirty"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { .. });
        assert!(db.is_live(a30));
        put(db, "alice", "email", lit("a@x.org"), Valid::ALWAYS);
        let bb = put(db, "bob", "email", lit("b@x.org"), Valid::ALWAYS);
        let r = db.try_tx(|tx| tx.assert(iri("bob"), iri("email"), lit("a@x.org"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::UniqueViolation { .. });
        assert!(db.is_live(bb));
    }
}

// predicate-schema "Typed layer accepts a statement subject" / "Typed layer rejects a
// node subject" / "Several values mean any-of" / "Subject type takes effect within its
// transaction" / "Failed subject type prevents cardinality replacement"
// @lat: [[tests#Typed Layers#Subject Type Constrains Subjects]]
host_test! {
    fn subject_type_constrains_subjects(db) {
        let e1 = put(db, "alice", "worksAt", iri("acme"), Valid::ALWAYS);
        flag(db, "confidence", "subjectType", sys("STMT"));
        let (conf, stmt) = (db.id(&iri("confidence")), db.id(&sys("STMT")));
        db.tx(|tx| tx.assert(e1, iri("confidence"), Value::Double(0.8), Valid::ALWAYS).map(|_| ()));
        let before = db.snapshot();
        let r = db.try_tx(|tx| tx.assert(iri("alice"), iri("confidence"), Value::Double(0.8), Valid::ALWAYS).map(|_| ()));
        match r {
            Err(e @ Error::SubjectTypeMismatch { .. }) => {
                let msg = e.to_string();
                assert!(msg.contains("subject type mismatch") && msg.contains("IRI"), "{msg}");
                let Error::SubjectTypeMismatch { p, expected, got } = e else { unreachable!() };
                assert_eq!((p, expected, got), (conf, vec![stmt], Tag::Iri));
            }
            other => panic!("expected SubjectTypeMismatch, got {other:?}"),
        }
        let r = db.try_tx(|tx| tx.create(iri("bob"), iri("confidence"), Value::Double(0.5), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SubjectTypeMismatch { .. });
        assert_eq!(db.snapshot(), before);
        // several values mean any-of; a second value retracts nothing
        let fs = flag(db, "note", "subjectType", sys("STMT"));
        let ft = flag(db, "note", "subjectType", sys("TX"));
        assert!(db.is_live(fs) && db.is_live(ft));
        let again = db.tx(|tx| {
            let r = tx.assert(iri("note"), sys("subjectType"), sys("TX"), Valid::ALWAYS)?;
            assert!(!r.is_new());
            Ok(())
        });
        assert!(again.retracted.is_empty());
        db.tx(|tx| {
            tx.assert(e1, iri("note"), lit("x"), Valid::ALWAYS)?;
            tx.meta(iri("note"), lit("y")).map(|_| ())
        });
        let r = db.try_tx(|tx| tx.assert(iri("alice"), iri("note"), lit("z"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SubjectTypeMismatch { expected, got: Tag::Iri, .. } if expected.len() == 2);
        // takes effect within its transaction
        let r = db.try_tx(|tx| {
            tx.assert(iri("score"), sys("subjectType"), sys("STMT"), Valid::ALWAYS)?;
            tx.assert(iri("alice"), iri("score"), Value::Int(1), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(r, Error::SubjectTypeMismatch { .. });
        // checked after the value type and before cardinality-one replacement
        db.tx(|tx| {
            tx.assert(iri("rank"), sys("cardinality"), sys("one"), Valid::ALWAYS)?;
            tx.assert(iri("rank"), sys("subjectType"), sys("STMT"), Valid::ALWAYS)?;
            tx.assert(iri("rank"), sys("valueType"), Value::iri(XSD_INTEGER), Valid::ALWAYS)?;
            Ok(())
        });
        let r1 = put(db, "x", "worksAt", iri("y"), Valid::ALWAYS);
        let mut ranked = None;
        db.tx(|tx| {
            ranked = Some(tx.assert(r1, iri("rank"), Value::Int(1), Valid::ALWAYS)?.eid());
            Ok(())
        });
        let r = db.try_tx(|tx| tx.assert(iri("alice"), iri("rank"), lit("one"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ValueTypeMismatch { .. });
        let r = db.try_tx(|tx| tx.assert(iri("alice"), iri("rank"), Value::Int(2), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SubjectTypeMismatch { .. });
        assert!(db.is_live(ranked.unwrap()));
    }
}

// predicate-schema "Subject type that no subject can have" / "A second subject type is
// added"
// @lat: [[tests#Typed Layers#Subject Type Values Are Validated]]
host_test! {
    fn subject_type_values_are_validated(db) {
        for ok in ["IRI", "NODE", "BNODE", "STMT", "TX"] {
            flag(db, "any", "subjectType", sys(ok));
        }
        let (any, st) = (db.id(&iri("any")), db.id(&sys("subjectType")));
        assert_eq!(db.now(Some(any), Some(st), None).len(), 5);
        for bad in [sys("INT"), sys("NOPE"), Value::iri(XSD_INTEGER), lit("STMT")] {
            let r = db.try_tx(|tx| tx.assert(iri("confidence"), sys("subjectType"), bad, Valid::ALWAYS).map(|_| ()));
            assert_err!(r, Error::ValueTypeMismatch { p, .. } if p == st);
        }
        let r = db.try_tx(|tx| tx.assert(sys("reason"), sys("subjectType"), sys("TX"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::ReservedNamespace(_));
    }
}

// predicate-schema "Subject type over node-level data" / "Retracting one of several
// subject types narrows the set" / "Retracting the last subject type lifts the
// constraint"
// @lat: [[tests#Typed Layers#Subject Type Changes Are Checked Against Live Data]]
host_test! {
    fn subject_type_schema_changes(db) {
        let e1 = put(db, "alice", "worksAt", iri("acme"), Valid::ALWAYS);
        let mut on_stmt = None;
        db.tx(|tx| {
            on_stmt = Some(tx.assert(e1, iri("confidence"), Value::Double(0.8), Valid::ALWAYS)?.eid());
            Ok(())
        });
        let on_node = put(db, "alice", "confidence", Value::Double(0.5), Valid::ALWAYS);
        let before = db.snapshot();
        let r = db.try_tx(|tx| tx.assert(iri("confidence"), sys("subjectType"), sys("STMT"), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SchemaConflict { violating } if violating == vec![on_node]);
        assert_eq!(db.snapshot(), before);
        // a second value only widens the set
        db.tx(|tx| tx.retract(on_node).map(|_| ()));
        let fs = flag(db, "confidence", "subjectType", sys("STMT"));
        let fi = flag(db, "confidence", "subjectType", sys("IRI"));
        let on_iri = put(db, "bob", "confidence", Value::Double(0.4), Valid::ALWAYS);
        // retracting one of several narrows the set, which live data violates
        let r = db.try_tx(|tx| tx.retract(fi).map(|_| ()));
        assert_err!(r, Error::SchemaConflict { violating } if violating == vec![on_iri]);
        let (conf, st, stmt) = (db.id(&iri("confidence")), db.id(&sys("subjectType")), db.id(&sys("STMT")));
        let r = db.try_tx(|tx| tx.retract_matching(Some(conf), Some(st), Some(stmt)).map(|_| ()));
        assert_err!(r, Error::SchemaConflict { violating } if violating == vec![on_stmt.unwrap()]);
        // superseding a value checks the set without the old value
        let r = db.try_tx(|tx| tx.supersede(fi, Patch::object(sys("TX"))).map(|_| ()));
        assert_err!(r, Error::SchemaConflict { violating } if violating == vec![on_iri]);
        // once the data conforms, narrowing is allowed
        db.tx(|tx| {
            tx.retract(on_iri)?;
            tx.retract(fi).map(|_| ())
        });
        let r = db.try_tx(|tx| tx.assert(iri("carol"), iri("confidence"), Value::Double(0.1), Valid::ALWAYS).map(|_| ()));
        assert_err!(r, Error::SubjectTypeMismatch { .. });
        // retracting the last value lifts the constraint, and the schema read earlier
        // in the transaction is not reused
        db.tx(|tx| {
            tx.assert(e1, iri("confidence"), Value::Double(0.9), Valid::ALWAYS)?;
            tx.retract(fs)?;
            tx.assert(iri("carol"), iri("confidence"), Value::Double(0.1), Valid::ALWAYS).map(|_| ())
        });
    }
}
