//! `cypher-temporal-clauses`: USE AS OF / VALID AT / HISTORY, per-pattern scopes,
//! statement time properties.
#![cfg(feature = "cypher")]
mod cypher_common;
use cypher_common::*;
use tiramemsu::*;

const DAY: i64 = 86_400_000;
fn date_ms(y: i64, m: i64, d: i64) -> i64 {
    // 1970-01-01 based; only used for 2020..2030
    let mut days = 0;
    for yy in 1970..y {
        days += if yy % 4 == 0 { 366 } else { 365 };
    }
    let dm = [
        31,
        if y % 4 == 0 { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for mm in 1..m {
        days += dm[(mm - 1) as usize];
    }
    (days + d - 1) * DAY
}

/// alice worked at acme from tx 3, superseded at tx 7 by globex.
fn job_change() -> T {
    let t = T::new();
    t.advance_to(2);
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]); // tx 3
    t.advance_to(6);
    let old = t.db.now().triples(None, None, None).unwrap()[0].eid;
    t.tx(|tx| {
        tx.supersede(old, Patch::object(v("globex")))?;
        Ok(())
    }); // tx 7
    t
}

// "Past episodes visible by default"
#[test]
fn past_episodes_visible_by_default() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(date_ms(2020, 1, 1), date_ms(2022, 1, 1)),
        )?;
        Ok(())
    });
    let r = t.q("MATCH (a)-[:worksAt]->(c) RETURN c");
    assert_eq!(short(&r.rows[0][0]), "acme");
}

// "Superseded value as of an earlier tx" / "Before the first transaction" / "Parameterised tx" / "Float tx rejected"
#[test]
fn use_as_of_a_transaction() {
    let t = job_change();
    let q =
        |n: u32| format!("USE AS OF {n} MATCH (a {{`@id`: 'v:alice'}})-[:worksAt]->(c) RETURN c");
    assert_eq!(short(&t.q(&q(5)).rows[0][0]), "acme");
    assert_eq!(short(&t.q(&q(7)).rows[0][0]), "globex");
    assert_eq!(t.one("USE AS OF 0 MATCH (n) RETURN count(n) AS c"), i(0));
    let r = t.qp(
        "USE AS OF $t MATCH (n) RETURN count(n)",
        &params(&[("t", i(5))]),
    );
    assert_eq!(r.rows, t.q("USE AS OF 5 MATCH (n) RETURN count(n)").rows);
    match t.qerr("USE AS OF 5.5 MATCH (n) RETURN n") {
        Error::Parse { span, .. } => assert_eq!(span.unwrap().offset, 10),
        other => panic!("{other:?}"),
    }
    let e =
        t.db.now()
            .cypher("USE AS OF $t MATCH (n) RETURN n", &params(&[("t", f(1.5))]))
            .unwrap_err();
    assert!(matches!(e, Error::Eval { .. }), "{e:?}");
}

// "Instant resolves to a transaction" / "Offset of the instant is honoured" / "Instant before any transaction"
#[test]
fn use_as_of_an_instant() {
    let t = T::new();
    let base = date_ms(2026, 9, 1);
    t.clock.set(base + 10 * 3_600_000);
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]); // tx 1 at 10:00Z
    t.clock.set(base + 12 * 3_600_000);
    let old = t.db.now().triples(None, None, None).unwrap()[0].eid;
    t.tx(|tx| {
        tx.supersede(old, Patch::object(v("globex")))?;
        Ok(())
    }); // tx 2 at 12:00Z
    let base_q = "MATCH (a {`@id`: 'v:alice'})-[:worksAt]->(c) RETURN c";
    for at in [
        "datetime('2026-09-01T11:00:00Z')",
        "datetime('2026-09-01T13:00:00+02:00')",
    ] {
        let r = t.q(&format!("USE AS OF {at} {base_q}"));
        assert_eq!(short(&r.rows[0][0]), "acme", "{at}");
    }
    assert_eq!(
        t.one("USE AS OF datetime('1999-01-01T00:00:00Z') MATCH (n) RETURN count(n) AS c"),
        i(0)
    );
    let r = t.qp(
        &format!("USE AS OF $at {base_q}"),
        &params(&[(
            "at",
            CypherValue::DateTime {
                ms: base + 11 * 3_600_000,
                tz: 0,
            },
        )]),
    );
    assert_eq!(short(&r.rows[0][0]), "acme");
}

// "Half-open interval" / "Unbounded statement always valid" / "Integer argument rejected"
#[test]
fn use_valid_at() {
    let t = T::new();
    t.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(date_ms(2025, 1, 1), date_ms(2026, 3, 1)),
        )?;
        tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        Ok(())
    });
    let q = |d: &str| format!("USE VALID AT date('{d}') MATCH (a)-[:worksAt]->(c) RETURN a");
    let names = |r: CypherResult| r.rows.iter().map(|x| short(&x[0])).collect::<Vec<_>>();
    assert_eq!(names(t.q(&q("2026-03-01"))), vec!["bob"]);
    let mut n = names(t.q(&q("2026-02-28")));
    n.sort();
    assert_eq!(n, vec!["alice", "bob"]);
    assert!(matches!(
        t.qerr("USE VALID AT 1700000000000 MATCH (n) RETURN n"),
        Error::Parse { .. }
    ));
}

// "All versions of a relationship"
#[test]
fn use_history() {
    let t = job_change();
    let r = t.q("USE HISTORY MATCH (a {`@id`: 'v:alice'})-[r:worksAt]->(c) \
         RETURN c, r.txAdded AS added, r.txRetracted AS gone ORDER BY added");
    assert_eq!(r.rows.len(), 2);
    assert_eq!(
        (
            short(&r.rows[0][0]),
            r.rows[0][1].clone(),
            r.rows[0][2].clone()
        ),
        ("acme".into(), i(3), i(7))
    );
    assert_eq!(
        (
            short(&r.rows[1][0]),
            r.rows[1][1].clone(),
            r.rows[1][2].clone()
        ),
        ("globex".into(), i(7), null())
    );
}

// "AS OF with VALID AT" / "Conflicting selectors" / "USE in the middle" / "Graph name rejected"
#[test]
fn combined_selectors_and_placement() {
    let t = T::new();
    t.assert(&[(v("alice"), v("worksAt"), v("acme"))]);
    assert_eq!(
        t.q("USE AS OF 1 VALID AT date('2021-06-01') MATCH (a)-[:worksAt]->(c) RETURN c")
            .rows
            .len(),
        1
    );
    assert!(matches!(
        t.qerr("USE AS OF 5 HISTORY MATCH (n) RETURN n"),
        Error::Parse { .. }
    ));
    match t.qerr("MATCH (n) USE AS OF 3 RETURN n") {
        Error::Parse { span, .. } => assert_eq!(span.unwrap().offset, 10),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        t.qerr("USE memory MATCH (n) RETURN n"),
        Error::Unsupported { .. }
    ));
}

// "What changed since tx 150" / "Property read under the scope's view" / "Nested scope inherits"
#[test]
fn per_pattern_scopes() {
    let t = job_change();
    let r = t.q("MATCH (a {`@id`: 'v:alice'})-[:worksAt]->(after) \
         CALL { WITH a USE AS OF 5 MATCH (a)-[:worksAt]->(before) RETURN before } \
         WITH before, after WHERE before <> after RETURN before, after");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(
        (short(&r.rows[0][0]), short(&r.rows[0][1])),
        ("acme".into(), "globex".into())
    );
    // property read under the scope's view
    let t2 = T::new();
    t2.assert(&[(v("alice"), v("name"), sv("Alicia"))]); // 1
    let old = t2.db.now().triples(None, None, None).unwrap()[0].eid;
    t2.tx(|tx| {
        tx.supersede(old, Patch::object(sv("Alice")))?;
        Ok(())
    }); // 2
    let r = t2.q(
        "MATCH (a {`@id`: 'v:alice'}) CALL { WITH a USE AS OF 1 RETURN a.name AS old } RETURN a.name AS now, old",
    );
    assert_eq!(r.rows[0], vec![s("Alice"), s("Alicia")]);
    // nested scope inherits
    let r =
        t.q("CALL { USE AS OF 5 CALL { MATCH (a)-[:worksAt]->(c) RETURN c } RETURN c } RETURN c");
    assert_eq!(short(&r.rows[0][0]), "acme");
}

// "Query AS OF overrides the handle AS OF" / "Handle valid time kept"
#[test]
fn in_query_clauses_override_the_handle_view() {
    let t = job_change();
    let past = t.db.as_of(TimeRef::Tx(4));
    let r = past
        .cypher(
            "USE AS OF 7 MATCH (a)-[:worksAt]->(c) RETURN c",
            &no_params(),
        )
        .unwrap();
    assert_eq!(short(&r.rows[0][0]), "globex");
    let t2 = T::new();
    t2.tx(|tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(date_ms(2021, 1, 1), date_ms(2022, 1, 1)),
        )?;
        tx.assert(
            v("bob"),
            v("worksAt"),
            v("acme"),
            Valid::between(date_ms(2023, 1, 1), date_ms(2024, 1, 1)),
        )?;
        Ok(())
    });
    let view = t2.db.now().valid_at(date_ms(2021, 6, 1));
    let a = view
        .cypher(
            "USE AS OF 1 MATCH (a)-[:worksAt]->(c) RETURN a",
            &no_params(),
        )
        .unwrap();
    let b_ = t2.q("USE AS OF 1 VALID AT date('2021-06-01') MATCH (a)-[:worksAt]->(c) RETURN a");
    assert_eq!(a.rows.len(), 1);
    assert_eq!(a.rows, b_.rows);
}

/// e1 asserted at tx 3 with valid interval [2025-01-01, open), with `confidence`.
fn timed() -> (T, Eid) {
    let t = T::new();
    t.advance_to(2);
    let mut e1 = None;
    t.tx(|tx| {
        e1 = Some(
            tx.assert(
                v("alice"),
                v("worksAt"),
                v("acme"),
                Valid::from(date_ms(2025, 1, 1)),
            )?
            .eid(),
        );
        tx.assert(
            e1.unwrap(),
            v("confidence"),
            Value::Double(0.8),
            Valid::ALWAYS,
        )?;
        tx.assert(v("belief9"), rdf_type(), v("Belief"), Valid::ALWAYS)?;
        tx.assert(v("belief9"), v("SUPPORTED_BY"), e1.unwrap(), Valid::ALWAYS)?;
        Ok(())
    });
    (t, e1.unwrap())
}

// "Time metadata of a live relationship" / "Same metadata on the node form" / "Time metadata not listed as keys"
#[test]
fn statement_time_properties() {
    let (t, _) = timed();
    let r = t.q("MATCH (a)-[r:worksAt]->(c) RETURN r.txAdded, r.txRetracted, r.validFrom, r.validTo, keys(r)");
    assert_eq!(r.rows[0][0], i(3));
    assert_eq!(r.rows[0][1], null());
    assert_eq!(r.to_json()["rows"][0][2], "2025-01-01T00:00:00.000Z");
    assert_eq!(r.rows[0][3], null());
    assert_eq!(r.rows[0][4], list(vec![s("confidence")]));
    assert_eq!(
        t.one("MATCH (:Belief)-[:SUPPORTED_BY]->(x) RETURN x.txAdded"),
        i(3)
    );
    let CypherValue::List(_) = &r.rows[0][4] else {
        panic!()
    };
}

// "Retract kind in history"
#[test]
fn retract_kind_in_history() {
    let (t, e1) = timed();
    t.tx(|tx| {
        tx.retract(e1)?;
        Ok(())
    });
    let r = t.q("USE HISTORY MATCH ()-[r]->() WHERE r.txRetracted IS NOT NULL RETURN r.`tm:retractKind` AS k, type(r) AS t");
    assert!(
        r.rows.contains(&vec![s("explicit"), s("worksAt")]),
        "{:?}",
        r.rows
    );
    assert!(r.rows.iter().any(|x| x[0] == s("cascade")), "{:?}", r.rows);
}

// "Shadowed user property reachable by CURIE" / "Ordinary node key"
#[test]
fn shadowed_and_ordinary_names() {
    let (t, e1) = timed();
    t.assert(&[(Value::Stmt(e1), v("txAdded"), sv("custom"))]);
    let r = t.q("MATCH ()-[r:worksAt]->() RETURN r.txAdded, r.`v:txAdded`");
    assert_eq!(r.rows[0], vec![i(3), s("custom")]);
    t.assert(&[(v("alice"), v("validFrom"), sv("someday"))]);
    assert_eq!(
        t.one("MATCH (n {`@id`: 'v:alice'}) RETURN n.validFrom"),
        s("someday")
    );
}

// "Close an open interval" / "txAdded is read-only" / "Inverted interval"
#[test]
fn set_valid_time_on_a_statement() {
    let (t, e1) = timed();
    let r = t.w("MATCH ()-[r:worksAt]->() SET r.validTo = date('2026-03-01') RETURN r.validTo AS t, elementId(r) AS id");
    assert_eq!(r.to_json()["rows"][0][0], "2026-03-01T00:00:00.000Z");
    let CypherValue::String(id) = &r.rows[0][1] else {
        panic!()
    };
    assert_ne!(id, &format!("urn:tiramemsu:stmt:{}", e1.n()));
    let rep = r.report.unwrap();
    assert!(rep
        .retracted
        .iter()
        .any(|(e, k)| *e == e1 && *k == RetKind::Supersede));
    assert!(matches!(
        t.werr("MATCH ()-[r:worksAt]->() SET r.txAdded = 1"),
        Error::Unsupported { .. }
    ));
    let before = t.last_t();
    assert!(matches!(
        t.werr("MATCH ()-[r:worksAt]->() SET r.validTo = date('2024-01-01')"),
        Error::InvalidPatch(_)
    ));
    assert_eq!(t.last_t(), before);
}

/// `e1 = (alice worksAt acme)` with `confidence`, referenced by `belief9`, all
/// committed at 2026-03-10T00:00:00Z.
fn instants() -> (T, Eid) {
    let t = T::new();
    t.clock.set(date_ms(2026, 3, 10));
    let mut e1 = None;
    t.tx(|tx| {
        let e = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(e, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
        tx.assert(v("belief9"), rdf_type(), v("Belief"), Valid::ALWAYS)?;
        tx.assert(v("belief9"), v("SUPPORTED_BY"), e, Valid::ALWAYS)?;
        e1 = Some(e);
        Ok(())
    });
    (t, e1.unwrap())
}

// cypher-temporal-clauses "Commit instants of a relationship" / "Retraction instant in
// history" / "Same metadata on the node form" / "Time metadata not listed as keys" /
// "Shadowed user property reachable by CURIE" / "addedAt is read-only"
// @lat: [[tests#Query#Cypher Statement Instants]]
#[test]
fn statement_instants() {
    let (t, e1) = instants();
    let r = t.q("MATCH (a)-[r:worksAt]->(c) RETURN r.addedAt, r.retractedAt, keys(r)");
    let j = r.to_json();
    assert_eq!(j["rows"][0][0], "2026-03-10T00:00:00.000Z");
    assert_eq!(r.rows[0][1], null());
    assert_eq!(r.rows[0][2], list(vec![s("confidence")]));
    let node = t.q("MATCH (:Belief)-[:SUPPORTED_BY]->(x) RETURN x.addedAt, x.`tm:addedAt`");
    assert_eq!(node.to_json()["rows"][0][0], "2026-03-10T00:00:00.000Z");
    assert_eq!(node.rows[0][0], node.rows[0][1]);
    // a relationship property map tests the metadata
    let hit = "MATCH ()-[r:worksAt {addedAt: datetime('2026-03-10T02:00:00+02:00')}]->() \
               RETURN count(r)";
    assert_eq!(t.one(hit), i(1));
    let miss = "MATCH ()-[r:worksAt {addedAt: datetime('2026-03-11T00:00:00Z')}]->() \
                RETURN count(r)";
    assert_eq!(t.one(miss), i(0));
    assert_eq!(
        t.one("MATCH ()-[r:worksAt {txAdded: 1}]->() RETURN count(r)"),
        i(1)
    );
    // transaction time is read-only, and nothing is written
    let before = t.last_t();
    for text in [
        "MATCH ()-[r:worksAt]->() SET r.addedAt = datetime()",
        "MATCH ()-[r:worksAt]->() REMOVE r.retractedAt",
    ] {
        assert!(matches!(t.werr(text), Error::Unsupported { .. }), "{text}");
    }
    assert_eq!(t.last_t(), before);
    // a stored property of the same name stays reachable by CURIE
    t.assert(&[(Value::Stmt(e1), v("addedAt"), sv("custom"))]);
    let r = t.q("MATCH ()-[r:worksAt]->() RETURN r.addedAt, r.`v:addedAt`, keys(r)");
    assert_eq!(r.to_json()["rows"][0][0], "2026-03-10T00:00:00.000Z");
    assert_eq!(r.rows[0][1], s("custom"));
    assert_eq!(r.rows[0][2], list(vec![s("confidence")]));
    // the retraction instant, visible in history
    t.clock.set(date_ms(2026, 3, 12));
    t.tx(|tx| tx.retract(e1).map(|_| ()));
    let r = t.q("USE HISTORY MATCH ()-[r:worksAt]->() RETURN r.retractedAt");
    assert_eq!(r.to_json()["rows"][0][0], "2026-03-12T00:00:00.000Z");
}

// cypher-temporal-clauses "Learned late by more than N days" / "Recorded after it
// stopped being true in Cypher"
// @lat: [[tests#Query#Bitemporal Recipes In Cypher]]
#[test]
fn learned_late_and_recorded_after_the_fact() {
    let t = T::new();
    t.clock.set(date_ms(2026, 3, 10));
    t.tx(|tx| {
        let at = |a: &str, b: &str| (v(a), v("worksAt"), v(b));
        for ((s, p, o), valid) in [
            (at("alice", "acme"), Valid::from(date_ms(2026, 3, 9))),
            (at("bob", "acme"), Valid::from(date_ms(2026, 4, 1))),
            (
                at("carol", "initech"),
                Valid::between(date_ms(2025, 1, 1), date_ms(2025, 6, 1)),
            ),
            (
                at("dave", "initech"),
                Valid::between(date_ms(2025, 1, 1), date_ms(2027, 1, 1)),
            ),
        ] {
            tx.assert(s, p, o, valid)?;
        }
        Ok(())
    });
    let late = |days: i64| -> Vec<String> {
        t.qp(
            "MATCH (a)-[r:worksAt]->() \
             WHERE r.addedAt.epochMillis - r.validFrom.epochMillis > $days * 86400000 \
             RETURN a ORDER BY elementId(a)",
            &params(&[("days", i(days))]),
        )
        .rows
        .iter()
        .map(|r| short(&r[0]))
        .collect()
    };
    assert_eq!(late(30), ["carol", "dave"]);
    assert_eq!(late(0), ["alice", "carol", "dave"]);
    let after = t.q("MATCH (a)-[r:worksAt]->() WHERE r.addedAt > r.validTo RETURN a");
    assert_eq!(after.rows.len(), 1);
    assert_eq!(short(&after.rows[0][0]), "carol");
}

// @lat: [[tests#Named Graphs#Cypher Keeps One Graph]]
#[test]
fn use_graph_is_unsupported() {
    let t = job_change();
    let before = t.last_t();
    for text in [
        "USE GRAPH g1 MATCH (n) RETURN n",
        "USE g1 MATCH (n) RETURN n",
    ] {
        match t.qerr(text) {
            Error::Unsupported { feature } => assert_eq!(feature, "USE GRAPH", "{text}"),
            other => panic!("{text}: {other:?}"),
        }
    }
    assert_eq!(t.last_t(), before);
    // the time forms of USE are unaffected
    assert_eq!(t.one("USE AS OF 0 MATCH (n) RETURN count(n) AS c"), i(0));
}
