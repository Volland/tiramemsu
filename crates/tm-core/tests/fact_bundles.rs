//! Spec `fact-bundles`: building a bundle from a view and importing it elsewhere.

mod common;
use common::*;

use tm_core::*;

fn bundle(db: &mut TestDb, spec: ViewSpec, root: Eid) -> Result<Bundle> {
    db.store().read(|e| read::bundle(e, &spec, root))
}

fn import(db: &mut TestDb, b: &Bundle) -> Result<(TxReport, ImportReport)> {
    let mut out = None;
    let rep = db.try_tx(|tx| {
        out = Some(tx.import_bundle(b)?);
        Ok(())
    })?;
    Ok((rep, out.unwrap()))
}

fn val(v: Value) -> BTerm {
    BTerm::Value(v)
}

/// A fact with layers, a nested layer, a belief that relies on it, a membership
/// with its own layer, a confirmation and an unrelated statement about alice.
struct Memory {
    e1: Eid,
    e2: Eid,
    e3: Eid,
    e7: Eid,
    e8: Eid,
    m: Eid,
    added_by: Eid,
}

fn memory(db: &mut TestDb) -> Memory {
    let mut out = None;
    db.tx(|tx| {
        let job = Valid::between(day("2020-01-01"), day("2024-01-01"));
        let e1 = tx
            .assert(iri("alice"), iri("worksAt"), iri("acme"), job)?
            .eid();
        let e2 = tx
            .assert(e1, iri("confidence"), Value::Double(0.8), Valid::ALWAYS)?
            .eid();
        let e3 = tx
            .assert(e2, iri("source"), lit("chat"), Valid::ALWAYS)?
            .eid();
        let e7 = tx
            .assert(iri("belief9"), iri("supportedBy"), e1, Valid::ALWAYS)?
            .eid();
        let e8 = tx
            .assert(e7, iri("method"), lit("llm-extraction"), Valid::ALWAYS)?
            .eid();
        let opts = AssertOpts {
            valid: Valid::from(day("2021-01-01")),
            ..AssertOpts::default()
        };
        let (m, _) = tx.add_to_graph(e1, iri("session12"), opts)?;
        let added_by = tx
            .assert(m, iri("addedBy"), iri("agent7"), Valid::ALWAYS)?
            .eid();
        tx.assert(iri("alice"), iri("name"), lit("Alice"), Valid::ALWAYS)?;
        out = Some(Memory {
            e1,
            e2,
            e3,
            e7,
            e8,
            m,
            added_by,
        });
        Ok(())
    });
    db.tx(|tx| tx.confirm(out.as_ref().unwrap().e1).map(|_| ()));
    out.unwrap()
}

// @lat: [[tests#Fact Bundles#Bundle Collects Layers And Evidence]]
host_test! {
    fn members_order_and_stability(db) {
        let mem = memory(db);
        let b = assert_ok(bundle(db, ViewSpec::NOW, mem.e1));
        // the dependents, confirmation left out, alice's name not walked
        let rows: Vec<Triple> = [mem.e1, mem.e2, mem.e7, mem.m, mem.e3, mem.e8, mem.added_by]
            .iter()
            .map(|e| db.row(*e))
            .collect();
        assert_eq!(b.statements.len(), rows.len());
        assert_eq!(b.root_statement().unwrap().p, iri("worksAt"));
        // references first, ties by eid: e1, then its layers by eid, then theirs
        let preds: Vec<Value> = b.statements.iter().map(|s| s.p.clone()).collect();
        assert_eq!(
            preds,
            [iri("worksAt"), iri("confidence"), iri("source"), iri("supportedBy"), iri("method"), sys("inGraph"), iri("addedBy")]
        );
        for (k, st) in b.statements.iter().enumerate() {
            assert_eq!(st.local, k as u32);
            for t in [&st.s, &st.o] {
                if let BTerm::Stmt(r) = t {
                    assert!(*r < st.local, "{st:?} references a later statement");
                }
            }
        }
        assert_eq!(b.statements[1].s, BTerm::Stmt(0));
        assert_eq!(b.statements[1].o, val(Value::Double(0.8)));
        assert_eq!(b.statements[0].valid, Valid::between(day("2020-01-01"), day("2024-01-01")));
        assert_eq!(b.statements[5].o, val(iri("session12")));
        assert_eq!(b.statements[5].valid, Valid::from(day("2021-01-01")));
        // the belief brings its evidence downward, but not the evidence's other layers
        let belief = assert_ok(bundle(db, ViewSpec::NOW, mem.e7));
        let preds: Vec<Value> = belief.statements.iter().map(|s| s.p.clone()).collect();
        assert_eq!(preds, [iri("worksAt"), iri("supportedBy"), iri("method")]);
        assert_eq!(belief.root, 1);
        // the same view of the same data gives an equal bundle
        assert_eq!(assert_ok(bundle(db, ViewSpec::NOW, mem.e1)), b);
        // a root that is not visible
        db.tx(|tx| tx.retract(mem.e8).map(|_| ()));
        assert_err!(bundle(db, ViewSpec::NOW, mem.e8), Error::NotLive(e) if e == mem.e8);
        assert_err!(bundle(db, ViewSpec::NOW, Eid::new(9_999)), Error::NotLive(_));
    }
}

// @lat: [[tests#Fact Bundles#Bundle Leaves Out Local Statements]]
host_test! {
    fn exclusions(db) {
        // a confirmation and a layer on it; transaction metadata
        let mut ids = None;
        db.tx(|tx| {
            let e1 = tx.assert(iri("alice"), iri("age"), Value::Int(30), Valid::ALWAYS)?.eid();
            let note = tx.assert(e1, iri("source"), iri("form"), Valid::ALWAYS)?.eid();
            let author = tx.meta(sys("author"), iri("agent7"))?;
            ids = Some((e1, note, author));
            Ok(())
        });
        let (e1, note, author) = ids.unwrap();
        let c = db.tx(|tx| tx.confirm(e1).map(|_| ())).asserted[0];
        db.tx(|tx| tx.assert(c, iri("note"), lit("seen twice"), Valid::ALWAYS).map(|_| ()));
        let b = assert_ok(bundle(db, ViewSpec::NOW, e1));
        assert_eq!(b.statements.len(), 2);
        assert!(b.statements.iter().all(|s| s.p != sys("confirmedBy") && s.p != iri("note")));
        assert_err!(bundle(db, ViewSpec::NOW, author), Error::Unsupported { feature } if feature.contains("transaction"));
        assert_err!(bundle(db, ViewSpec::NOW, c), Error::Unsupported { .. });

        // a supersede replays the layer; the link to the old eid stays behind
        let rep = db.tx(|tx| tx.supersede(e1, Patch::object(Value::Int(31))).map(|_| ()));
        let (_, e10) = rep.superseded[0];
        let b = assert_ok(bundle(db, ViewSpec::NOW, e10));
        let preds: Vec<Value> = b.statements.iter().map(|s| s.p.clone()).collect();
        assert_eq!(preds, [iri("age"), iri("source")]);
        assert_eq!(b.statements[0].o, val(Value::Int(31)));
        assert!(!db.is_live(note));

        // a statement that names a statement outside the view, and what stands on it
        let mut ids = None;
        let dead = db.tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ())).asserted[0];
        db.tx(|tx| tx.retract(dead).map(|_| ()));
        db.tx(|tx| {
            let r = tx.assert(iri("c"), iri("p"), iri("d"), Valid::ALWAYS)?.eid();
            let x = tx.create(r, iri("links"), dead, Valid::ALWAYS)?;
            let y = tx.create(x, iri("note"), lit("on x"), Valid::ALWAYS)?;
            ids = Some((r, x, y));
            Ok(())
        });
        let (r, x, _) = ids.unwrap();
        let b = assert_ok(bundle(db, ViewSpec::NOW, r));
        assert_eq!(b.statements.len(), 1);
        assert_err!(
            bundle(db, ViewSpec::NOW, x),
            Error::Unsupported { feature } if feature.contains("not in the view")
        );
    }
}

// @lat: [[tests#Fact Bundles#Bundle Round Trip Between Databases]]
host_test! {
    fn round_trip(db) {
        let mem = memory(db);
        let b = assert_ok(bundle(db, ViewSpec::NOW, mem.e1));
        let mut target = TestDb::new(db.kind);
        let (rep, imported) = assert_ok(import(&mut target, &b));
        assert!(imported.statements.iter().all(|s| s.new));
        assert_eq!(rep.asserted.len(), 6);
        assert_eq!(rep.memberships.len(), 1);
        let root = imported.root;
        assert_eq!(imported.eid(b.root), Some(root));
        // the target's bundle of the imported root is the same bundle: same contents,
        // same layer structure, same membership, same valid times
        assert_eq!(assert_ok(bundle(&mut target, ViewSpec::NOW, root)), b);
        let g = target.id(&iri("session12"));
        assert_eq!(target.read(|e| read::graph_members(e, &ViewSpec::NOW, g)), vec![root]);
        let job = target.row(root);
        assert_eq!(job.valid(), Valid::between(day("2020-01-01"), day("2024-01-01")));
        // the confirmation (a transaction of the source file) did not travel
        let works_at = target.id(&iri("worksAt"));
        assert_eq!(target.now(None, Some(works_at), None).len(), 1);
        assert!(target.oid(&sys("confirmedBy")).is_none());
    }
}

// @lat: [[tests#Fact Bundles#Import Is Idempotent]]
host_test! {
    fn second_import_changes_nothing(db) {
        let mem = memory(db);
        let b = assert_ok(bundle(db, ViewSpec::NOW, mem.e7));
        let mut target = TestDb::new(db.kind);
        let (_, first) = assert_ok(import(&mut target, &b));
        let before = target.data_snapshot();
        let (rep, second) = assert_ok(import(&mut target, &b));
        assert!(second.statements.iter().all(|s| !s.new));
        assert_eq!(
            second.statements.iter().map(|s| s.eid).collect::<Vec<_>>(),
            first.statements.iter().map(|s| s.eid).collect::<Vec<_>>()
        );
        assert!(rep.asserted.is_empty() && rep.memberships.is_empty() && rep.retracted.is_empty());
        // only the new transaction row differs
        let after = target.data_snapshot();
        for ((t, a), (_, b)) in before.iter().zip(&after) {
            if t != "tx" {
                assert_eq!(a, b, "table {t}");
            }
        }
    }
}

// @lat: [[tests#Fact Bundles#Import Attaches To An Existing Root]]
host_test! {
    fn existing_root_fact(db) {
        let mem = memory(db);
        let b = assert_ok(bundle(db, ViewSpec::NOW, mem.e1));
        let mut target = TestDb::new(db.kind);
        let existing = target
            .tx(|tx| tx.assert(iri("alice"), iri("worksAt"), iri("acme"), Valid::ALWAYS).map(|_| ()))
            .asserted[0];
        let (_, imported) = assert_ok(import(&mut target, &b));
        assert_eq!(imported.root, existing);
        assert!(!imported.statements[b.root as usize].new);
        assert!(imported.statements.iter().filter(|s| s.local != b.root).all(|s| s.new));
        // the layers stand on the existing eid, whose valid time is not changed
        let deps = target.read(|e| read::dependents(e, &ViewSpec::NOW, existing));
        assert_eq!(deps.len(), b.statements.len());
        assert_eq!(target.row(existing).valid(), Valid::ALWAYS);
        let _ = (mem.e2, mem.e3, mem.e8, mem.added_by);
    }
}

// @lat: [[tests#Fact Bundles#Anonymous Nodes Stay Distinct]]
host_test! {
    fn anonymous_nodes(db) {
        let mut root = None;
        db.tx(|tx| {
            let n1 = tx.new_node()?;
            let n2 = tx.new_bnode()?;
            let e1 = tx.assert(n1, iri("worksAt"), iri("acme"), Valid::ALWAYS)?.eid();
            tx.assert(e1, iri("witnessedBy"), n2, Valid::ALWAYS)?;
            tx.assert(e1, iri("about"), n1, Valid::ALWAYS)?;
            root = Some(e1);
            Ok(())
        });
        let b = assert_ok(bundle(db, ViewSpec::NOW, root.unwrap()));
        assert_eq!(b.statements[0].s, BTerm::Node(0));
        assert_eq!(b.statements[1].o, BTerm::Node(1));
        assert_eq!(b.statements[2].o, BTerm::Node(0));
        let mut target = TestDb::new(db.kind);
        // the target already uses node ids, which the bundle must not alias
        target.tx(|tx| {
            let n = tx.new_node()?;
            tx.assert(n, iri("p"), iri("q"), Valid::ALWAYS).map(|_| ())
        });
        let (_, first) = assert_ok(import(&mut target, &b));
        let row = |db: &mut TestDb, k: usize| db.row(first.statements[k].eid);
        let (s0, o1, o2) = (row(&mut target, 0).s, row(&mut target, 1).o, row(&mut target, 2).o);
        assert_eq!(s0.tag().unwrap(), Tag::Node);
        assert_eq!(o1.tag().unwrap(), Tag::Node);
        assert_eq!(s0, o2);
        assert_ne!(s0, o1);
        let p = target.id(&iri("p"));
        let old = target.now(None, Some(p), None)[0].s;
        assert!(old != s0 && old != o1);
        // blank-node semantics: a second import mints new nodes, so it is new again
        let (_, second) = assert_ok(import(&mut target, &b));
        assert!(second.statements.iter().all(|s| s.new));
        assert_ne!(target.row(second.statements[0].eid).s, s0);
    }
}

// @lat: [[tests#Fact Bundles#Import Rejects Cycles And Malformed Bundles]]
host_test! {
    fn cycles_and_malformed(db) {
        let n = db.meta("next_stmt") as u64;
        let e7 = db
            .tx(|tx| {
                let e7 = tx.create(iri("b1"), iri("about"), Eid::new(n + 1), Valid::ALWAYS)?;
                tx.create(e7, iri("about"), iri("b2"), Valid::ALWAYS)?;
                Ok(())
            })
            .asserted[0];
        // export keeps the cycle; import refuses it before writing
        let b = assert_ok(bundle(db, ViewSpec::NOW, e7));
        assert_eq!(b.statements.len(), 2);
        let mut target = TestDb::new(db.kind);
        let before = target.snapshot();
        assert_err!(
            import(&mut target, &b),
            Error::Unsupported { feature } if feature == "bundle with a reference cycle"
        );
        assert_eq!(target.snapshot(), before);

        let st = |local, s, o| BundleStatement { local, s, p: iri("p"), o, valid: Valid::ALWAYS };
        let bad = [
            // a skolem IRI would alias a statement of the target
            Bundle { root: 0, statements: vec![st(0, val(iri("x")), val(Value::iri("urn:tiramemsu:stmt:3")))] },
            Bundle { root: 0, statements: vec![st(0, val(iri("x")), val(Value::Node(4)))] },
            // a reference to a statement the bundle does not hold
            Bundle { root: 0, statements: vec![st(0, BTerm::Stmt(7), val(iri("y")))] },
            // duplicate ids, unknown root
            Bundle { root: 0, statements: vec![st(0, val(iri("x")), val(iri("y"))), st(0, val(iri("x")), val(iri("z")))] },
            Bundle { root: 5, statements: vec![st(0, val(iri("x")), val(iri("y")))] },
            // a literal predicate
            Bundle { root: 0, statements: vec![BundleStatement { p: lit("p"), ..st(0, val(iri("x")), val(iri("y"))) }] },
        ];
        for b in &bad {
            assert_err!(import(&mut target, b), Error::InvalidTerm { .. });
        }
        // a self-reference is a cycle of one
        let selfref = Bundle { root: 0, statements: vec![st(0, BTerm::Stmt(0), val(iri("y")))] };
        assert_err!(import(&mut target, &selfref), Error::Unsupported { .. });
        assert_eq!(target.snapshot(), before);
    }
}

// @lat: [[tests#Fact Bundles#Import Fails Atomically On Schema]]
host_test! {
    fn schema_violation_is_atomic(db) {
        let root = db
            .tx(|tx| {
                let e = tx.assert(iri("alice"), iri("email"), lit("x@example.org"), Valid::ALWAYS)?.eid();
                tx.assert(e, iri("source"), lit("form"), Valid::ALWAYS)?;
                Ok(())
            })
            .asserted[0];
        let b = assert_ok(bundle(db, ViewSpec::NOW, root));
        let mut target = TestDb::new(db.kind);
        target.tx(|tx| {
            tx.assert(iri("email"), sys("unique"), Value::Bool(true), Valid::ALWAYS)?;
            tx.assert(iri("bob"), iri("email"), lit("x@example.org"), Valid::ALWAYS)?;
            Ok(())
        });
        let before = target.data_snapshot();
        assert_err!(import(&mut target, &b), Error::UniqueViolation { .. });
        assert_eq!(target.data_snapshot(), before);
    }
}

// @lat: [[tests#Fact Bundles#Bundle Of The Past]]
host_test! {
    fn as_of_bundle_of_a_retracted_structure(db) {
        let mem = memory(db);
        let t = db.last_t();
        db.tx(|tx| tx.retract(mem.e1).map(|_| ()));
        assert_err!(bundle(db, ViewSpec::NOW, mem.e1), Error::NotLive(_));
        let then = assert_ok(bundle(db, ViewSpec::as_of(TimeRef::Tx(t)), mem.e1));
        assert_eq!(then.statements.len(), 7);
        let mut target = TestDb::new(db.kind);
        let (_, imported) = assert_ok(import(&mut target, &then));
        // live copies of the whole structure
        assert_eq!(assert_ok(bundle(&mut target, ViewSpec::NOW, imported.root)), then);
        assert!(imported.statements.iter().all(|s| target.is_live(s.eid)));
    }
}

mod random {
    use super::*;
    use proptest::prelude::*;

    fn proptest_cases(default: u32) -> u32 {
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// Random layered graphs in which no two live statements share their content
    /// (import collapses those, as assert does): assertions are idempotent and
    /// correction objects are unique per transaction.
    #[derive(Clone, Debug)]
    enum GenOp {
        Base { s: u8, o: u8, from: Option<i64> },
        Layer { target: u16, o: u8 },
        Reference { s: u8, target: u16 },
        Link { a: u16, b: u16 },
        Member { target: u16, g: u8 },
        Confirm { target: u16 },
        Supersede { target: u16, o: u8 },
        Retract { target: u16 },
    }

    fn op_strategy() -> impl Strategy<Value = GenOp> {
        prop_oneof![
            3 => (0u8..4, 0u8..4, prop::option::of(0i64..1000))
                .prop_map(|(s, o, from)| GenOp::Base { s, o, from }),
            4 => (any::<u16>(), 0u8..4).prop_map(|(target, o)| GenOp::Layer { target, o }),
            2 => (0u8..4, any::<u16>()).prop_map(|(s, target)| GenOp::Reference { s, target }),
            2 => (any::<u16>(), any::<u16>()).prop_map(|(a, b)| GenOp::Link { a, b }),
            1 => (any::<u16>(), 0u8..2).prop_map(|(target, g)| GenOp::Member { target, g }),
            1 => any::<u16>().prop_map(|target| GenOp::Confirm { target }),
            1 => (any::<u16>(), 0u8..4).prop_map(|(target, o)| GenOp::Supersede { target, o }),
            2 => any::<u16>().prop_map(|target| GenOp::Retract { target }),
        ]
    }

    fn pick(known: &[Eid], i: u16) -> Option<Eid> {
        (!known.is_empty()).then(|| known[i as usize % known.len()])
    }

    fn apply(tx: &mut Tx<'_>, op: &GenOp, known: &[Eid]) -> Result<()> {
        let node = |i: u8| iri(&format!("n{i}"));
        match *op {
            GenOp::Base { s, o, from } => {
                let valid = from.map_or(Valid::ALWAYS, Valid::from);
                tx.assert(node(s), iri("p"), node(o), valid)?;
            }
            GenOp::Layer { target, o } => {
                if let Some(e) = pick(known, target) {
                    tx.assert(e, iri("note"), Value::Int(o as i64), Valid::ALWAYS)?;
                }
            }
            GenOp::Reference { s, target } => {
                if let Some(e) = pick(known, target) {
                    tx.assert(node(s), iri("cites"), e, Valid::ALWAYS)?;
                }
            }
            GenOp::Link { a, b } => {
                if let (Some(a), Some(b)) = (pick(known, a), pick(known, b)) {
                    if a != b {
                        tx.assert(a, iri("links"), b, Valid::ALWAYS)?;
                    }
                }
            }
            GenOp::Member { target, g } => {
                if let Some(e) = pick(known, target) {
                    tx.add_to_graph(e, node(g), AssertOpts::default())?;
                }
            }
            GenOp::Confirm { target } => {
                if let Some(e) = pick(known, target) {
                    tx.confirm(e)?;
                }
            }
            GenOp::Supersede { target, o } => {
                if let Some(e) = pick(known, target) {
                    // unique per transaction, so two corrections never share content
                    let fresh = 100 + 4 * tx.t().0 as i64 + o as i64;
                    tx.supersede(e, Patch::object(Value::Int(fresh)))?;
                }
            }
            GenOp::Retract { target } => {
                if let Some(e) = pick(known, target) {
                    tx.retract(e)?;
                }
            }
        }
        Ok(())
    }

    // Export, import into an empty file, export again: the same bundle; and a
    // second import is a no-op.
    // @lat: [[tests#Fact Bundles#Bundles Round Trip On Random Graphs]]
    #[test]
    fn prop_bundles_round_trip() {
        let cfg = ProptestConfig::with_cases(proptest_cases(24));
        proptest!(cfg, |(ops in prop::collection::vec(op_strategy(), 1..30), minimal in any::<bool>())| {
            let kind = if minimal { HostKind::Minimal } else { HostKind::Rusqlite };
            let mut db = TestDb::new(kind);
            let mut known: Vec<Eid> = Vec::new();
            for op in &ops {
                let snapshot = known.clone();
                if let Ok(rep) = db.try_tx(|tx| apply(tx, op, &snapshot)) {
                    known.extend(rep.asserted.iter().copied());
                    known.extend(rep.memberships.iter().copied());
                }
            }
            for e in db.now_eids() {
                let b = match bundle(&mut db, ViewSpec::NOW, e) {
                    Ok(b) => b,
                    // a root local to this file (a confirmation, a supersede link)
                    Err(Error::Unsupported { .. }) => continue,
                    Err(other) => panic!("bundle of {e:?}: {other:?}"),
                };
                let mut target = TestDb::new(kind);
                let (_, first) = import(&mut target, &b).expect("import");
                prop_assert!(first.statements.iter().all(|s| s.new));
                let again = bundle(&mut target, ViewSpec::NOW, first.root).expect("re-export");
                prop_assert_eq!(again, b.clone());
                let (rep, second) = import(&mut target, &b).expect("second import");
                prop_assert!(rep.asserted.is_empty() && rep.memberships.is_empty());
                let unchanged: Vec<ImportedStatement> = first
                    .statements
                    .iter()
                    .map(|s| ImportedStatement { new: false, ..*s })
                    .collect();
                prop_assert_eq!(second.statements, unchanged);
            }
        });
    }
}
