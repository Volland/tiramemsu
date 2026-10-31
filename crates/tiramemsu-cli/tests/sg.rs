//! `tm export --format sg-nt12`: each store below is exported in each of its views and
//! compared byte for byte with `tests/sg/<store>.<view>.nt`. LeanRDF records the same
//! files (`scripts/statements-tm-record.sh` there) and checks that each decodes in its
//! view and exports back byte for byte. `TM_SG_UPDATE=1 cargo test -p tiramemsu-cli
//! --test sg` rewrites them after an intended change; review the diff.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tiramemsu::*;

fn tm(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tm"))
        .args(args)
        .output()
        .expect("tm runs")
}

fn ex(s: &str) -> Value {
    Value::iri(format!("http://example.org/{s}"))
}

/// One transaction of a test store: gets the eids returned so far, returns new ones.
type Step<'a> = &'a dyn Fn(&mut Tx<'_>, &[Eid]) -> Result<Vec<Eid>>;

/// A new SQLite store in `dir`, filled by `fill` one transaction per closure.
fn store(dir: &Path, fill: &[Step<'_>]) -> PathBuf {
    let f = dir.join("m.db");
    let db = Db::create(&f, Backend::Sqlite, OpenOptions::default()).unwrap();
    let mut eids = Vec::new();
    for step in fill {
        let mut new = Vec::new();
        db.transact(TxOptions::default(), |tx| {
            new = step(tx, &eids)?;
            Ok(())
        })
        .unwrap();
        eids.extend(new);
    }
    f
}

/// Exports `file` with `flags` and compares with (or, with `TM_SG_UPDATE`, writes)
/// `tests/sg/<name>.nt`.
fn golden(file: &Path, name: &str, flags: &[&str]) {
    let mut args = vec!["export", file.to_str().unwrap(), "--format", "sg-nt12"];
    args.extend(flags);
    let o = tm(&args);
    assert!(
        o.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let got = String::from_utf8(o.stdout).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/sg/{name}.nt"));
    if std::env::var_os("TM_SG_UPDATE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap();
    } else {
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(got, want, "{name} differs from {}", path.display());
    }
}

// @tg: verifies:: [[openspec:tm-cli#Statement graph export#A route and its distance]] {since: "2026-10-31", level: e2e}
#[test]
fn routes() {
    let d = tempfile::tempdir().unwrap();
    let f = store(
        d.path(),
        &[
            &|tx, _| {
                Ok(vec![tx
                    .assert(ex("FRA"), ex("route"), ex("LHR"), Valid::ALWAYS)?
                    .eid()])
            },
            &|tx, e| {
                Ok(vec![tx
                    .assert(e[0], ex("distance"), Value::Int(655), Valid::ALWAYS)?
                    .eid()])
            },
        ],
    );
    golden(&f, "routes.now", &[]);
    golden(&f, "routes.history", &["--history"]);
}

// @tg: verifies:: [[openspec:tm-cli#Statement graph export#A correction in four views]] {since: "2026-10-31", level: e2e}
#[test]
fn correction() {
    let d = tempfile::tempdir().unwrap();
    let f = store(
        d.path(),
        &[
            &|tx, _| {
                Ok(vec![tx
                    .assert(ex("FRA"), ex("route"), ex("LHR"), Valid::ALWAYS)?
                    .eid()])
            },
            &|tx, e| {
                Ok(vec![tx
                    .assert(e[0], ex("distance"), Value::Int(665), Valid::ALWAYS)?
                    .eid()])
            },
            &|tx, _| {
                let summer = Valid::between(1_774_828_800_000, 1_792_800_000_000);
                Ok(vec![tx
                    .assert(ex("LHR"), ex("route"), ex("JFK"), summer)?
                    .eid()])
            },
            &|tx, e| {
                let fixed = tx.supersede(e[1], Patch::object(Value::Int(655)))?;
                tx.assert(fixed, ex("source"), ex("airlineDB"), Valid::ALWAYS)?;
                Ok(vec![fixed])
            },
        ],
    );
    golden(&f, "correction.now", &[]);
    golden(&f, "correction.asof3", &["--as-of", "3"]);
    golden(&f, "correction.history", &["--history"]);
    golden(&f, "correction.valid-at", &["--valid-at", "1800000000000"]);
}

// @tg: verifies:: [[openspec:tm-cli#Statement graph export#Never-visible statements are left out]] {since: "2026-10-31", level: e2e}
#[test]
fn never_visible() {
    let d = tempfile::tempdir().unwrap();
    let f = store(
        d.path(),
        &[&|tx, _| {
            let gone = tx.assert(ex("a"), ex("p"), ex("b"), Valid::ALWAYS)?.eid();
            tx.retract(gone)?;
            Ok(vec![tx
                .assert(ex("a"), ex("p"), ex("c"), Valid::ALWAYS)?
                .eid()])
        }],
    );
    golden(&f, "never-visible.history", &["--history"]);
}

// @tg: verifies:: [[openspec:tm-cli#Statement graph export#Canonical RDF 1.2 N-Triples]] {since: "2026-10-31", level: e2e}
#[test]
fn literals() {
    let d = tempfile::tempdir().unwrap();
    let f = store(
        d.path(),
        &[&|tx, _| {
            let a = tx.assert(
                ex("a"),
                ex("note"),
                Value::str("tab\there \"q\" \u{8}\u{c}\u{1}\u{7f}"),
                Valid::ALWAYS,
            )?;
            tx.assert(
                ex("a"),
                ex("label"),
                Value::LangStr {
                    lex: "Grüße".into(),
                    lang: "DE".into(),
                },
                Valid::ALWAYS,
            )?;
            tx.assert(ex("a"), ex("on"), Value::Bool(true), Valid::ALWAYS)?;
            tx.assert(
                a.eid(),
                ex("confidence"),
                Value::Decimal("0.9".into()),
                Valid::ALWAYS,
            )?;
            Ok(vec![])
        }],
    );
    golden(&f, "literals.now", &[]);
}

#[test]
fn view_flags_need_sg_nt12() {
    let d = tempfile::tempdir().unwrap();
    let f = store(d.path(), &[]);
    let o = tm(&["export", f.to_str().unwrap(), "--history"]);
    assert!(!o.status.success());
}
