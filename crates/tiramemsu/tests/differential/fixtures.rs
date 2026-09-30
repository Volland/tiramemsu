//! Named fixtures of the differential suite: sequences of transactions applied
//! through the Rust API under a deterministic clock (never through a query dialect).

use std::sync::Arc;

use tiramemsu::*;

/// 2026-09-01T00:00:00Z in epoch milliseconds.
pub const BASE: i64 = 1_788_220_800_000;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 86_400_000;

/// The epoch milliseconds of midnight UTC of `y-m-d` (2020..2030).
pub fn date_ms(y: i64, m: i64, d: i64) -> i64 {
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

/// `urn:tiramemsu:v:local`.
pub fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

fn ty() -> Value {
    Value::iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
}

fn s(x: &str) -> Value {
    Value::str(x)
}

fn dt(lex: &str) -> Value {
    Value::literal(lex, Some(vocab::XSD_DATETIME), None)
}

/// A fixture loaded into a fresh database.
pub struct Fx {
    pub _dir: tempfile::TempDir,
    pub db: Db,
    pub clock: Arc<ManualClock>,
}

impl Fx {
    fn tx<F: FnOnce(&mut Tx<'_>) -> Result<()>>(&self, at: i64, f: F) {
        self.clock.set(at);
        self.db
            .transact(TxOptions::default(), f)
            .expect("fixture transaction");
    }

    fn tick(&self, at: i64) {
        self.tx(at, |_| Ok(()));
    }
}

/// The names of the fixtures.
pub const NAMES: [&str; 9] = [
    "paths",
    "employment",
    "parallel",
    "layers",
    "superseded",
    "cascade",
    "cardinality",
    "episodes",
    "literals",
];

/// Loads the fixture `name` into a fresh database.
pub fn load(name: &str) -> Fx {
    let dir = tempfile::tempdir().expect("tempdir");
    let clock = Arc::new(ManualClock::new(BASE));
    let db = Db::open(
        dir.path().join("d.db"),
        OpenOptions {
            clock: clock.clone(),
            readers: 0,
            ..OpenOptions::default()
        },
    )
    .expect("open");
    let fx = Fx {
        _dir: dir,
        db,
        clock,
    };
    match name {
        "employment" => employment(&fx),
        "paths" => paths(&fx),
        "parallel" => parallel(&fx),
        "layers" => layers(&fx),
        "superseded" => superseded(&fx),
        "cascade" => cascade(&fx),
        "cardinality" => cardinality(&fx),
        "episodes" => episodes(&fx),
        "literals" => literals(&fx),
        other => panic!("unknown fixture {other}"),
    }
    fx
}

/// A small social and employment graph with labels and literal properties.
fn employment(fx: &Fx) {
    fx.tx(BASE, |tx| {
        for (p, name, age) in [
            ("alice", "Alice", Some(41)),
            ("bob", "Bob", Some(30)),
            ("carol", "Carol", Some(25)),
            ("dave", "Dave", None),
        ] {
            tx.assert(v(p), ty(), v("Person"), Valid::ALWAYS)?;
            tx.assert(v(p), v("name"), s(name), Valid::ALWAYS)?;
            if let Some(a) = age {
                tx.assert(v(p), v("age"), Value::Int(a), Valid::ALWAYS)?;
            }
        }
        for p in ["alice", "bob"] {
            tx.assert(v(p), ty(), v("Employee"), Valid::ALWAYS)?;
        }
        for (c, name) in [("acme", "Acme"), ("initech", "Initech")] {
            tx.assert(v(c), ty(), v("Company"), Valid::ALWAYS)?;
            tx.assert(v(c), v("name"), s(name), Valid::ALWAYS)?;
        }
        for (a, p, o) in [
            ("alice", "worksAt", "acme"),
            ("bob", "worksAt", "acme"),
            ("carol", "worksAt", "initech"),
            ("alice", "knows", "bob"),
            ("bob", "knows", "carol"),
            ("acme", "locatedIn", "berlin"),
            ("initech", "locatedIn", "paris"),
        ] {
            tx.assert(v(a), v(p), v(o), Valid::ALWAYS)?;
        }
        Ok(())
    });
}

/// A cycle with a parallel edge and a tail: alice knows bob, bob knows carol twice,
/// carol knows alice and dave; later dave knows erin.
fn paths(fx: &Fx) {
    fx.tx(BASE, |tx| {
        for (n, name) in [
            ("alice", "Alice"),
            ("bob", "Bob"),
            ("carol", "Carol"),
            ("dave", "Dave"),
            ("erin", "Erin"),
        ] {
            tx.assert(v(n), ty(), v("Person"), Valid::ALWAYS)?;
            tx.assert(v(n), v("name"), s(name), Valid::ALWAYS)?;
        }
        tx.create(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("bob"), v("knows"), v("carol"), Valid::ALWAYS)?;
        tx.create(v("bob"), v("knows"), v("carol"), Valid::ALWAYS)?;
        tx.create(v("carol"), v("knows"), v("alice"), Valid::ALWAYS)?;
        tx.create(v("carol"), v("knows"), v("dave"), Valid::ALWAYS)?;
        Ok(())
    });
    fx.tx(BASE + HOUR, |tx| {
        tx.create(v("dave"), v("knows"), v("erin"), Valid::ALWAYS)?;
        Ok(())
    });
}

/// Parallel edges made by `create`, one plain edge and a multi-valued property.
fn parallel(fx: &Fx) {
    fx.tx(BASE, |tx| {
        tx.assert(v("alice"), ty(), v("Person"), Valid::ALWAYS)?;
        tx.assert(v("bob"), ty(), v("Person"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.create(v("alice"), v("called"), v("bob"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?;
        Ok(())
    });
    fx.tx(BASE + HOUR, |tx| {
        tx.assert(v("alice"), v("nick"), s("al"), Valid::ALWAYS)?;
        Ok(())
    });
    fx.tx(BASE + 2 * HOUR, |tx| {
        tx.assert(v("alice"), v("nick"), s("ally"), Valid::ALWAYS)?;
        Ok(())
    });
}

/// Layers: an annotation, a statement about a statement, two levels deep.
fn layers(fx: &Fx) {
    fx.tx(BASE, |tx| {
        tx.assert(v("alice"), ty(), v("Person"), Valid::ALWAYS)?;
        let e1 = tx
            .assert(
                v("alice"),
                v("WORKS_AT"),
                v("acme"),
                Valid::from(date_ms(2025, 1, 1)),
            )?
            .eid();
        tx.assert(e1, v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
        tx.assert(v("belief9"), ty(), v("Belief"), Valid::ALWAYS)?;
        let e7 = tx
            .assert(v("belief9"), v("SUPPORTED_BY"), e1, Valid::ALWAYS)?
            .eid();
        tx.assert(e7, v("method"), s("llm-extraction"), Valid::ALWAYS)?;
        tx.meta(Value::iri("urn:tiramemsu:sys:author"), v("agent7"))?;
        Ok(())
    });
}

/// alice worked at acme from tx 3 (10:00Z) until globex replaced it in tx 7 (12:00Z);
/// her name changed in tx 8.
fn superseded(fx: &Fx) {
    fx.tick(BASE + HOUR);
    fx.tick(BASE + 2 * HOUR);
    fx.tx(BASE + 10 * HOUR, |tx| {
        tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        tx.assert(v("alice"), v("name"), s("Alicia"), Valid::ALWAYS)?;
        Ok(())
    });
    for k in 4..7 {
        fx.tick(BASE + 10 * HOUR + k * 1000);
    }
    let old = fx
        .db
        .now()
        .triples(None, None, None)
        .unwrap()
        .into_iter()
        .find(|t| fx.db.now().decode(t.p).unwrap() == v("worksAt"))
        .unwrap()
        .eid;
    fx.tx(BASE + 12 * HOUR, |tx| {
        tx.supersede(old, Patch::object(v("globex")))?;
        Ok(())
    });
    let old = fx
        .db
        .now()
        .triples(None, None, None)
        .unwrap()
        .into_iter()
        .find(|t| fx.db.now().decode(t.p).unwrap() == v("name"))
        .unwrap()
        .eid;
    fx.tx(BASE + 13 * HOUR, |tx| {
        tx.supersede(old, Patch::object(s("Alice")))?;
        Ok(())
    });
}

/// A retraction with cascade: a relationship, its annotation and a statement about it.
fn cascade(fx: &Fx) {
    let mut e1 = None;
    fx.tx(BASE, |tx| {
        let e = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.assert(e, v("confidence"), Value::Double(0.9), Valid::ALWAYS)?;
        tx.assert(v("belief9"), v("SUPPORTED_BY"), e, Valid::ALWAYS)?;
        e1 = Some(e);
        Ok(())
    });
    fx.tx(BASE + HOUR, |tx| {
        tx.retract(e1.unwrap())?;
        Ok(())
    });
}

/// A cardinality-one replacement.
fn cardinality(fx: &Fx) {
    fx.tx(BASE, |tx| {
        tx.assert(
            v("age"),
            Value::iri("urn:tiramemsu:sys:cardinality"),
            Value::iri("urn:tiramemsu:sys:one"),
            Valid::ALWAYS,
        )?;
        tx.assert(v("alice"), v("age"), Value::Int(41), Valid::ALWAYS)?;
        Ok(())
    });
    fx.tx(BASE + HOUR, |tx| {
        tx.assert(v("alice"), v("age"), Value::Int(42), Valid::ALWAYS)?;
        Ok(())
    });
}

/// Valid-time episodes.
fn episodes(fx: &Fx) {
    fx.tx(BASE, |tx| {
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("acme"),
            Valid::between(date_ms(2025, 1, 1), date_ms(2026, 3, 1)),
        )?;
        tx.assert(
            v("alice"),
            v("worksAt"),
            v("globex"),
            Valid::from(date_ms(2026, 3, 1)),
        )?;
        tx.assert(v("bob"), v("worksAt"), v("acme"), Valid::ALWAYS)?;
        Ok(())
    });
}

/// Values of every literal type.
fn literals(fx: &Fx) {
    fx.tx(BASE, |tx| {
        let x = v("x");
        tx.assert(x.clone(), ty(), v("Thing"), Valid::ALWAYS)?;
        tx.assert(x.clone(), v("i"), Value::Int(7), Valid::ALWAYS)?;
        tx.assert(x.clone(), v("f"), Value::Double(1.5), Valid::ALWAYS)?;
        tx.assert(x.clone(), v("b"), Value::Bool(true), Valid::ALWAYS)?;
        tx.assert(x.clone(), v("s"), s("hello"), Valid::ALWAYS)?;
        tx.assert(
            x.clone(),
            v("d"),
            Value::literal("2025-03-01", Some(vocab::XSD_DATE), None),
            Valid::ALWAYS,
        )?;
        tx.assert(
            x.clone(),
            v("t"),
            dt("2026-03-01T12:00:00+02:00"),
            Valid::ALWAYS,
        )?;
        tx.assert(x.clone(), v("z"), dt("2026-03-01T10:00:00Z"), Valid::ALWAYS)?;
        tx.assert(x, v("l"), dt("2026-03-01T09:00:00"), Valid::ALWAYS)?;
        Ok(())
    });
}
