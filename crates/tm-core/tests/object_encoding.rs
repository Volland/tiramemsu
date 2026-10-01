//! Spec `object-encoding`: ObjectId layout, canonical encoding, round trip, order,
//! the term dictionary and skolem IRIs.

mod common;
use common::*;

use std::cell::RefCell;

use proptest::prelude::*;
use tm_core::id::COUNTER_MAX;
use tm_core::vocab::*;
use tm_core::*;

fn proptest_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Interns inside a committed transaction and returns the id.
fn intern(db: &mut TestDb, v: &Value) -> ObjectId {
    let mut id = None;
    let v = v.clone();
    db.tx(|tx| {
        id = Some(tx.encode(&v)?);
        Ok(())
    });
    id.unwrap()
}

fn term_row(db: &mut TestDb, id: ObjectId) -> (String, Option<i64>, Option<String>, Option<f64>) {
    let r = db.rows(&format!(
        "SELECT lex, dt, lang, num FROM term WHERE id = {}",
        id.unsigned_payload()
    ));
    let r = &r[0];
    (
        r[0].as_str().unwrap().to_string(),
        r[1].as_i64(),
        r[2].as_str().map(str::to_string),
        r[3].as_f64(),
    )
}

host_test! {
    /// ObjectId layout — Statement and transaction ids are inline.
    fn stmt_and_tx_ids_are_inline(db) {
        let before = db.count("term");
        let (e, t) = (intern(db, &Value::Stmt(Eid::new(42))), intern(db, &Value::Tx(TxId(7))));
        assert_eq!(e.raw(), (42 << 4) | 3);
        assert_eq!(t.raw(), (7 << 4) | 4);
        assert_eq!(db.count("term"), before);
        let five = intern(db, &Value::Int(5));
        assert_eq!(five.raw(), 85);
        assert_eq!(five.raw() & 15, 5);
    }
}

// Reserved SEALED tag and the reserved sys:sensitive flag (tasks 9.11 and
// object-encoding "Reserved SEALED tag is rejected", predicate-schema
// "Sensitive flag is reserved").
// @lat: [[tests#ObjectId#Reserved Tag Is Rejected]]
host_test! {
    fn reserved_tag_and_sensitive_flag_are_rejected(db) {
        db.tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        let before = db.snapshot();
        let sealed = ObjectId::from_raw((7 << 4) | 15);
        // decoding
        let r = TermReader::new(4);
        let got = db.read(|e| Ok(r.decode(e, sealed, false)));
        assert_err!(got, Error::Unsupported { feature } if feature.contains("SEALED") && feature.contains("M6"));
        // every write position
        let res = db.try_tx(|tx| tx.assert(iri("a"), iri("p"), sealed, Valid::ALWAYS).map(|_| ()));
        assert_err!(res, Error::Unsupported { feature } if feature.contains("SEALED"));
        let res = db.try_tx(|tx| tx.create(sealed, iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        assert_err!(res, Error::Unsupported { .. });
        let res = db.try_tx(|tx| tx.set_volatile(iri("a"), iri("k"), sealed));
        assert_err!(res, Error::Unsupported { .. });
        // sys:sensitive, whatever its subject and object
        let res = db.try_tx(|tx| {
            tx.assert(iri("x"), iri("q"), lit("a long string to intern"), Valid::ALWAYS)?;
            tx.assert(iri("email"), sys("sensitive"), Value::Bool(true), Valid::ALWAYS).map(|_| ())
        });
        assert_err!(res, Error::Unsupported { feature } if feature.contains("sys:sensitive") && feature.contains("M6"));
        let res = db.try_tx(|tx| tx.create(sys("db"), sys("sensitive"), lit("x"), Valid::ALWAYS).map(|_| ()));
        assert_err!(res, Error::Unsupported { .. });
        assert_eq!(db.snapshot(), before);
    }
}

host_test! {
    /// ObjectId layout — Local ids have origin 0: every id a format 1 file
    /// allocates has origin 0 and the same value as without origins.
    fn local_ids_have_origin_0(db) {
        let n = db.meta("next_stmt") as u64;
        let mut node = None;
        let r = db.tx(|tx| {
            let id = tx.new_node()?;
            node = Some(id);
            tx.assert(id, iri("p"), Value::Int(1), Valid::ALWAYS)?;
            Ok(())
        });
        let e = r.asserted[0];
        assert_eq!(e, Eid::new(n));
        assert_eq!(e.oid().raw(), ((n as i64) << 4) | 3);
        assert_eq!((e.oid().origin(), e.oid().counter()), (Some(0), Some(n)));
        let node = node.unwrap();
        assert_eq!((node.origin(), node.tag().unwrap()), (Some(0), Tag::Node));
        assert_eq!((r.t.oid().origin(), r.t.oid().counter()), (Some(0), Some(r.t.0)));
        // statement number 42 keeps the id of the layout without origins
        let e42 = intern(db, &Value::Stmt(Eid::new(42)));
        assert_eq!((e42.raw(), e42.origin()), ((42 << 4) | 3, Some(0)));
    }
}

// object-encoding "Foreign origin is rejected": a NODE, BNODE, STMT or TX id with
// a non-zero origin is refused on every input and by every write, leaving no trace.
// @lat: [[tests#ObjectId#Foreign Origin Is Rejected]]
host_test! {
    fn foreign_origin_is_rejected(db) {
        let local = db
            .tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()))
            .asserted[0];
        let before = db.snapshot();
        let payload = (1u64 << 48) | 5;
        let foreign = Eid::new(payload);
        let skolem = Value::iri(format!("urn:tiramemsu:stmt:{payload}"));
        let is_origin = |e: &Error| matches!(e, Error::Unsupported { feature } if feature.contains("origin 1"));
        let check = |r: Result<TxReport>| match r {
            Err(e) if is_origin(&e) => {}
            other => panic!("expected the origin to be rejected, got {other:?}"),
        };
        // the skolem IRI parses to the foreign statement, which is refused
        assert_eq!(skolem.canonical(), Value::Stmt(foreign));
        check(db.try_tx(|tx| tx.assert(skolem.clone(), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.encode(&skolem).map(|_| ())));
        // the id and its Value forms, in every position
        check(db.try_tx(|tx| tx.assert(foreign, iri("p"), iri("b"), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.assert(foreign.oid(), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.assert(iri("a"), iri("p"), Value::Stmt(foreign), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.create(Value::Node(payload), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.create(Value::BNode(payload), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.assert(iri("a"), iri("p"), Value::Tx(TxId(payload)), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.assert(iri("a"), iri("p"), TxId(payload), Valid::ALWAYS).map(|_| ())));
        check(db.try_tx(|tx| tx.set_volatile(Value::Node(payload), iri("k"), lit("v"))));
        // operations that name a statement
        check(db.try_tx(|tx| tx.retract(foreign).map(|_| ())));
        check(db.try_tx(|tx| tx.confirm(foreign).map(|_| ())));
        check(db.try_tx(|tx| tx.supersede(foreign, Patch { o: Some(iri("c")), ..Patch::default() }).map(|_| ())));
        check(db.try_tx(|tx| tx.add_to_graph(foreign, iri("g"), AssertOpts::default()).map(|_| ())));
        check(db.try_tx(|tx| tx.add_to_graph(local, Value::Node(payload), AssertOpts::default()).map(|_| ())));
        check(db.try_tx(|tx| tx.remove_from_graph(foreign, iri("g")).map(|_| ())));
        check(db.try_tx(|tx| tx.retract_matching(Some(foreign.oid()), None, None).map(|_| ())));
        // a failed write leaves no trace, even after earlier successful operations
        check(db.try_tx(|tx| {
            tx.assert(iri("x"), iri("q"), lit("a long string to intern"), Valid::ALWAYS)?;
            tx.assert(iri("x"), iri("q"), Value::Stmt(foreign), Valid::ALWAYS).map(|_| ())
        }));
        assert_eq!(db.snapshot(), before);
        // a read lookup refuses it too
        let got = db.read(|e| Ok(TermReader::encode(e, &skolem)));
        assert!(matches!(&got, Err(e) if is_origin(e)), "{got:?}");
        // bundle import
        let val = BTerm::Value;
        for (s, p, o) in [
            (val(iri("x")), iri("p"), val(skolem.clone())),
            (val(Value::Node(payload)), iri("p"), val(iri("y"))),
            (val(iri("x")), skolem.clone(), val(iri("y"))),
        ] {
            let b = Bundle {
                root: 0,
                statements: vec![BundleStatement { local: 0, s, p, o, valid: Valid::ALWAYS }],
            };
            check(db.try_tx(|tx| tx.import_bundle(&b).map(|_| ())));
        }
        assert_eq!(db.snapshot(), before);
        // the largest local counter is still accepted
        let last = Eid::new(COUNTER_MAX);
        assert!(last.oid().check_origin().is_ok());
        assert_eq!(intern(db, &Value::Stmt(last)), last.oid());
    }
}

host_test! {
    /// Canonical encoding of integers.
    fn integers(db) {
        let one = intern(db, &Value::Int(1));
        for lex in ["01", "+1", "1"] {
            assert_eq!(intern(db, &Value::literal(lex, Some(XSD_INTEGER), None)), one);
        }
        let before = db.count("term");
        let max = intern(db, &Value::Int((1 << 59) - 1));
        let min = intern(db, &Value::Int(-(1 << 59)));
        assert_eq!((max.tag().unwrap(), min.tag().unwrap()), (Tag::Int, Tag::Int));
        assert_eq!(db.count("term"), before);
        let xsd_int = intern(db, &Value::iri(XSD_INTEGER));
        for (v, lex) in [
            (Value::big_integer("576460752303423488"), "576460752303423488"),
            (Value::big_integer("-576460752303423489"), "-576460752303423489"),
            (Value::Int(1 << 59), "576460752303423488"),
        ] {
            let id = intern(db, &v);
            assert_eq!(id.tag().unwrap(), Tag::Typed);
            let (l, dt, _, _) = term_row(db, id);
            assert_eq!(l, lex);
            assert_eq!(dt, Some(xsd_int.raw()));
        }
        let derived = intern(db, &Value::literal("5", Some("http://www.w3.org/2001/XMLSchema#int"), None));
        assert_eq!(derived.tag().unwrap(), Tag::Typed);
        assert_ne!(derived, intern(db, &Value::Int(5)));
        assert_eq!(
            term_row(db, derived).1,
            Some(intern(db, &Value::iri("http://www.w3.org/2001/XMLSchema#int")).raw())
        );
    }
}

host_test! {
    /// Canonical encoding of booleans and dates.
    fn booleans_and_dates(db) {
        let t1 = intern(db, &Value::literal("1", Some(XSD_BOOLEAN), None));
        let t2 = intern(db, &Value::literal("true", Some(XSD_BOOLEAN), None));
        assert_eq!(t1, t2);
        assert_eq!((t1.tag().unwrap(), t1.unsigned_payload()), (Tag::Bool, 1));
        let d = intern(db, &Value::literal("1969-12-31", Some(XSD_DATE), None));
        assert_eq!((d.tag().unwrap(), d.signed_payload()), (Tag::Date, -1));
    }
}

// @lat: [[tests#ObjectId#DateTime Keeps Its Offset]]
host_test! {
    fn datetime_offsets_are_two_terms(db) {
        let a = intern(db, &Value::literal("2026-03-01T12:00:00+02:00", Some(XSD_DATETIME), None));
        let b = intern(db, &Value::literal("2026-03-01T10:00:00Z", Some(XSD_DATETIME), None));
        assert_eq!((a.tag().unwrap(), b.tag().unwrap()), (Tag::DateTime, Tag::DateTime));
        assert_ne!(a, b);
        assert_eq!(db.decode(a).lexical(), "2026-03-01T12:00:00.000+02:00");
        assert_eq!(db.decode(b).lexical(), "2026-03-01T10:00:00.000Z");
        // same instant: id >> 15 in Rust and in SQLite
        assert_eq!(a.instant(), b.instant());
        assert_eq!(a.raw() >> 15, b.raw() >> 15);
        let sql = db.rows(&format!("SELECT {} >> 15, {} >> 15", a.raw(), b.raw()));
        assert_eq!(sql[0][0], sql[0][1]);
        assert_eq!(sql[0][0], SqlValue::Integer(a.instant()));
    }
}

host_test! {
    /// Date-time without a timezone, and out of range.
    fn datetime_without_timezone_and_out_of_range(db) {
        let local = intern(db, &Value::literal("2026-03-01T10:00:00", Some(XSD_DATETIME), None));
        assert_eq!(local.signed_payload() & 0x7ff, 0);
        assert_eq!(db.decode(local).lexical(), "2026-03-01T10:00:00.000");
        let utc = intern(db, &Value::literal("2026-03-01T10:00:00Z", Some(XSD_DATETIME), None));
        assert_eq!(local.instant(), utc.instant());
        let far = "12000-01-01T00:00:00Z";
        let id = intern(db, &Value::literal(far, Some(XSD_DATETIME), None));
        assert_eq!(id.tag().unwrap(), Tag::Typed);
        let (lex, dt, _, _) = term_row(db, id);
        assert_eq!(lex, far);
        assert_eq!(dt, Some(intern(db, &Value::iri(XSD_DATETIME)).raw()));
        let off = intern(db, &Value::literal("2026-03-01T10:00:00+15:00", Some(XSD_DATETIME), None));
        assert_eq!(off.tag().unwrap(), Tag::Typed);
    }
}

host_test! {
    /// Canonical encoding of strings.
    fn strings(db) {
        let before = db.count("term");
        let short = intern(db, &lit("abcdefg"));
        assert_eq!(short.tag().unwrap(), Tag::ShortStr);
        assert_eq!(db.count("term"), before);
        let long = intern(db, &lit("abcdefgh"));
        assert_eq!(long.tag().unwrap(), Tag::Str);
        assert_eq!(db.count("term"), before + 1);
        assert_eq!(intern(db, &lit("héllo")).tag().unwrap(), Tag::ShortStr);
        assert_eq!(intern(db, &lit("€€€")).tag().unwrap(), Tag::Str);
        assert_eq!(
            intern(db, &lit("hello")),
            intern(db, &Value::literal("hello", Some(XSD_STRING), None))
        );
        let gb = intern(db, &Value::literal("colour", None, Some("en-GB")));
        let gb2 = intern(db, &Value::literal("colour", None, Some("en-gb")));
        assert_eq!(gb, gb2);
        assert_eq!(gb.tag().unwrap(), Tag::LangStr);
        assert_eq!(term_row(db, gb).2.as_deref(), Some("en-gb"));
        assert_eq!(intern(db, &Value::literal("hi", None, Some("en"))).tag().unwrap(), Tag::LangStr);
        for s in ["", "a\0b"] {
            let id = intern(db, &lit(s));
            assert_eq!(db.decode(id), lit(s));
        }
    }
}

host_test! {
    /// Canonical encoding of doubles, decimals and other datatypes.
    fn doubles_decimals_and_other_datatypes(db) {
        let a = intern(db, &Value::literal("1.0", Some(XSD_DOUBLE), None));
        let b = intern(db, &Value::literal("1E0", Some(XSD_DOUBLE), None));
        assert_eq!(a, b);
        assert_eq!(a.tag().unwrap(), Tag::Double);
        assert_eq!(term_row(db, a).3, Some(1.0));
        let d1 = intern(db, &Value::literal("1.50", Some(XSD_DECIMAL), None));
        let d2 = intern(db, &Value::literal("01.5", Some(XSD_DECIMAL), None));
        assert_eq!(d1, d2);
        assert_eq!(d1.tag().unwrap(), Tag::Decimal);
        assert_eq!(term_row(db, d1).3, Some(1.5));
        let wkt = "http://www.opengis.net/ont/geosparql#wktLiteral";
        let p = intern(db, &Value::literal("POINT(1 2)", Some(wkt), None));
        assert_eq!(p.tag().unwrap(), Tag::Typed);
        let (lex, dt, _, _) = term_row(db, p);
        assert_eq!(lex, "POINT(1 2)");
        assert_eq!(dt, Some(intern(db, &Value::iri(wkt)).raw()));
        let bad = intern(db, &Value::literal("abc", Some(XSD_INTEGER), None));
        assert_eq!(bad.tag().unwrap(), Tag::Typed);
        assert_eq!(term_row(db, bad).0, "abc");
        assert_eq!(
            db.decode(bad),
            Value::Typed { lex: "abc".into(), datatype: XSD_INTEGER.into() }
        );
        let nan = intern(db, &Value::Double(f64::NAN));
        assert_eq!(term_row(db, nan).3, None);
    }
}

fn one_per_tag() -> Vec<Value> {
    vec![
        Value::iri("https://example.org/alice"),
        Value::Node(3),
        Value::BNode(4),
        Value::Stmt(Eid::new(9)),
        Value::Tx(TxId(2)),
        Value::Int(-17),
        Value::Bool(true),
        Value::literal("2026-03-01T12:00:00.5+02:00", Some(XSD_DATETIME), None),
        Value::literal("2026-03-01", Some(XSD_DATE), None),
        lit("short"),
        lit("a longer plain string"),
        Value::literal("colour", None, Some("en-GB")),
        Value::literal("x", Some("urn:x:dt"), None),
        Value::Double(2.5e-3),
        Value::literal("-003.1400", Some(XSD_DECIMAL), None),
    ]
}

host_test! {
    /// Encode-decode round trip — Round trip for every tag.
    fn round_trip_every_tag(db) {
        let values = one_per_tag();
        let mut tags: Vec<Tag> = Vec::new();
        for v in &values {
            let id = intern(db, v);
            tags.push(id.tag().unwrap());
            let back = db.decode(id);
            assert_eq!(back, v.canonical(), "{v:?}");
            assert_eq!(intern(db, &back), id);
        }
        tags.sort();
        tags.dedup();
        assert_eq!(tags.len(), 15);
    }
}

fn value_strategy() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<i64>().prop_map(Value::Int),
        any::<bool>().prop_map(Value::Bool),
        (-(1i64 << 48)..(1i64 << 48), prop::option::of(-840i16..=840))
            .prop_map(|(ms, tz)| Value::DateTime { ms, tz }),
        (-3_000_000i64..3_000_000).prop_map(Value::Date),
        ".{0,20}".prop_map(Value::Str),
        (".{0,12}", "[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})?")
            .prop_map(|(lex, lang)| Value::LangStr { lex, lang }),
        any::<f64>()
            .prop_filter("not NaN", |x| !x.is_nan())
            .prop_map(Value::Double),
        (any::<i32>(), 0u32..1_000_000).prop_map(|(a, b)| Value::Decimal(format!("{a}.{b}"))),
        "[a-z]{1,12}".prop_map(|s| Value::iri(format!("urn:x:{s}"))),
        ("[a-z]{1,6}", "[a-z0-9 ]{0,10}").prop_map(|(d, lex)| Value::Typed {
            lex,
            datatype: format!("urn:dt:{d}")
        }),
        (1u64..1_000_000).prop_map(Value::Node),
        (1u64..1_000_000).prop_map(Value::BNode),
        (1u64..1_000_000).prop_map(|n| Value::Stmt(Eid::new(n))),
        (1u64..1_000_000).prop_map(|n| Value::Tx(TxId(n))),
    ]
}

// @lat: [[tests#ObjectId#Canonical Round Trip]]
#[test]
fn prop_canonical_round_trip() {
    for kind in [HostKind::Rusqlite, HostKind::Minimal] {
        let db = RefCell::new(TestDb::new(kind));
        let cfg = ProptestConfig::with_cases(proptest_cases(128));
        proptest!(cfg, |(v in value_strategy())| {
            let mut db = db.borrow_mut();
            let terms_before = db.count("term");
            let id = intern(&mut db, &v);
            let back = db.decode(id);
            let canon = v.canonical();
            match (&back, &canon) {
                (Value::Double(a), Value::Double(b)) => prop_assert_eq!(a.to_bits(), b.to_bits()),
                _ => prop_assert_eq!(&back, &canon),
            }
            prop_assert_eq!(intern(&mut db, &back), id);
            if !id.tag().unwrap().is_dictionary() {
                prop_assert_eq!(db.count("term"), terms_before);
            }
        });
    }
}

fn sqlite_lt(conn: &rusqlite::Connection, a: i64, b: i64, shift: bool) -> bool {
    let sql = if shift {
        "SELECT (?1 >> 15) < (?2 >> 15)"
    } else {
        "SELECT ?1 < ?2"
    };
    conn.query_row(sql, [a, b], |r| r.get::<_, bool>(0))
        .unwrap()
}

fn int_edge() -> impl Strategy<Value = i64> {
    prop_oneof![
        Just(-(1i64 << 59)),
        Just((1i64 << 59) - 1),
        Just(-1i64),
        Just(0i64),
        -(1i64 << 59)..(1i64 << 59),
    ]
}

fn ms_edge() -> impl Strategy<Value = i64> {
    prop_oneof![
        Just(-(1i64 << 48)),
        Just((1i64 << 48) - 1),
        Just(-1i64),
        Just(0i64),
        -(1i64 << 48)..(1i64 << 48),
    ]
}

fn oid(v: &Value) -> ObjectId {
    match codec::encode(v) {
        codec::Encoded::Inline(id) => id,
        e => panic!("{e:?}"),
    }
}

// @lat: [[tests#ObjectId#Order Within Tag]]
#[test]
fn prop_order_within_tag() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let ordered: Vec<i64> = [-1000i64, -1, 0, 1, (1 << 59) - 1]
        .iter()
        .map(|i| oid(&Value::Int(*i)).raw())
        .collect();
    assert!(ordered.windows(2).all(|w| w[0] < w[1]));
    let cfg = ProptestConfig::with_cases(proptest_cases(256));
    proptest!(cfg, |(a in int_edge(), b in int_edge(),
                     da in -(1i64 << 40)..(1i64 << 40), db_ in -(1i64 << 40)..(1i64 << 40),
                     ma in ms_edge(), mb in ms_edge(),
                     ta in prop::option::of(-840i16..=840), tb in prop::option::of(-840i16..=840))| {
        let (ia, ib) = (oid(&Value::Int(a)).raw(), oid(&Value::Int(b)).raw());
        prop_assert_eq!(a < b, ia < ib);
        prop_assert_eq!(a < b, sqlite_lt(&conn, ia, ib, false));
        let (xa, xb) = (oid(&Value::Date(da)).raw(), oid(&Value::Date(db_)).raw());
        prop_assert_eq!(da < db_, xa < xb);
        prop_assert_eq!(da < db_, sqlite_lt(&conn, xa, xb, false));
        let ya = oid(&Value::DateTime { ms: ma, tz: ta }).raw();
        let yb = oid(&Value::DateTime { ms: mb, tz: tb }).raw();
        prop_assert_eq!(ma < mb, (ya >> 15) < (yb >> 15));
        prop_assert_eq!(ma < mb, sqlite_lt(&conn, ya, yb, true));
        if ma < mb {
            prop_assert!(ya < yb);
            prop_assert!(sqlite_lt(&conn, ya, yb, false));
        }
        // ORDER BY over a mixed set equals value order
        let mut vals = vec![a, b, 0, -1];
        let ids: Vec<i64> = vals.iter().map(|v| oid(&Value::Int(*v)).raw()).collect();
        let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join("),(");
        let mut st = conn
            .prepare(&format!("SELECT column1 FROM (VALUES ({list})) ORDER BY column1"))
            .unwrap();
        let sorted: Vec<i64> = st
            .query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .map(|r| ObjectId::from_raw(r.unwrap()).signed_payload())
            .collect();
        vals.sort();
        prop_assert_eq!(sorted, vals);
    });
}

host_test! {
    /// Term dictionary deduplication and immutability.
    fn dictionary_dedupes(db) {
        let alice = Value::iri("https://example.org/alice");
        let a1 = intern(db, &alice);
        let a2 = intern(db, &alice);
        assert_eq!(a1, a2);
        assert_eq!(db.scalar("SELECT count(*) FROM term WHERE lex = 'https://example.org/alice'"), 1);
        let s = lit("twenty bytes string!");
        assert_eq!(intern(db, &s), intern(db, &s));
        assert_eq!(db.scalar("SELECT count(*) FROM term WHERE lex = 'twenty bytes string!'"), 1);
        let i = intern(db, &Value::iri("urn:x:abcdefghij"));
        let t = intern(db, &lit("urn:x:abcdefghij"));
        assert_ne!(i, t);
        assert_eq!((i.tag().unwrap(), t.tag().unwrap()), (Tag::Iri, Tag::Str));
        // term ids come from next_term
        let next = db.meta("next_term");
        let fresh = intern(db, &Value::iri("urn:x:fresh-one"));
        assert_eq!(fresh.unsigned_payload() as i64, next);
    }
}

host_test! {
    /// Fail after interning, then intern again: caches stay consistent with `term`.
    fn fail_after_interning_then_intern_again(db) {
        let v = Value::iri("urn:x:interned-in-a-failed-tx");
        let mut seen = None;
        let r = db.try_tx(|tx| {
            seen = Some(tx.encode(&v)?);
            Err(Error::custom("fail"))
        });
        assert!(r.is_err());
        assert_eq!(db.oid(&v), None);
        // another term takes the reissued id, then the value is interned again
        let other = intern(db, &Value::iri("urn:x:other"));
        assert_eq!(Some(other), seen);
        let again = intern(db, &v);
        assert_ne!(again, other);
        assert_eq!(db.oid(&v), Some(again));
        assert_eq!(db.decode(again), v);
        assert_eq!(db.decode(other), Value::iri("urn:x:other"));
    }
}

host_test! {
    /// Lookup without insertion on the read path.
    fn read_lookup_never_inserts(db) {
        db.tx(|tx| tx.assert(iri("a"), iri("p"), iri("b"), Valid::ALWAYS).map(|_| ()));
        let (terms, next) = (db.count("term"), db.meta("next_term"));
        assert_eq!(db.oid(&Value::iri("urn:x:never-written")), None);
        assert_eq!(db.oid(&lit("a never stored twenty-byte str")), None);
        assert_eq!((db.count("term"), db.meta("next_term")), (terms, next));
    }
}

host_test! {
    /// Skolem IRIs for anonymous nodes.
    fn skolem_iris(db) {
        let mut node = None;
        db.tx(|tx| {
            for _ in 0..12 {
                node = Some(tx.new_node()?);
            }
            Ok(())
        });
        let node = node.unwrap();
        assert_eq!(node.unsigned_payload(), 12);
        let exported = codec::skolem_iri(node).unwrap();
        assert_eq!(exported, "urn:tiramemsu:node:12");
        let terms = db.count("term");
        assert_eq!(intern(db, &Value::iri(&exported)), node);
        assert_eq!(db.count("term"), terms);
        let b = intern(db, &Value::iri("urn:tiramemsu:bnode:3"));
        assert_eq!((b.tag().unwrap(), b.unsigned_payload()), (Tag::BNode, 3));
        let odd = intern(db, &Value::iri("urn:tiramemsu:node:007"));
        assert_eq!(odd.tag().unwrap(), Tag::Iri);
        assert_eq!(db.count("term"), terms + 1);
    }
}
