//! Spec `sql-execution` "Host capabilities": the query engine refuses to open on a
//! host without `functions` or `vtab`, and works on the rusqlite host on pooled
//! readers and inside `with`.

mod common;

use common::probe::ProbeHost;
use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::ir::{Expr, Op};
use tiramemsu::*;

fn open_err(host: ProbeHost) -> Error {
    let d = tempfile::tempdir().unwrap();
    Db::open_with_host(host, d.path().join("h.db"), OpenOptions::default()).unwrap_err()
}

// sql-execution "Host without virtual tables"
#[test]
fn host_without_vtab() {
    match open_err(ProbeHost::new().with_caps(|c| c.vtab = false)) {
        Error::MissingCapability { capability } => assert_eq!(capability, "vtab"),
        e => panic!("{e:?}"),
    }
}

// sql-execution "Host without functions"
#[test]
fn host_without_functions() {
    match open_err(ProbeHost::new().with_caps(|c| c.functions = false)) {
        Error::MissingCapability { capability } => assert_eq!(capability, "functions"),
        e => panic!("{e:?}"),
    }
}

// sql-execution "First host has both"
#[test]
fn rusqlite_host_registers_everywhere() {
    let t = TestDb::open(OpenOptions {
        readers: 2,
        ..OpenOptions::default()
    });
    t.tx(|tx| {
        tx.assert(v("a"), v("n"), s("a long string value"), Valid::ALWAYS)?;
        tx.assert(v("b"), v("n"), s("Bo"), Valid::ALWAYS)?;
        Ok(())
    });
    let b = IrBuilder::sparql();
    let q = b.query(
        b.triple("?s", "v:n", "?x")
            .filter(Expr::lt(Expr::var("x"), Expr::val(s("Mzzzzzzzzz")))),
    );
    // pooled reader (both readers get used across calls)
    for _ in 0..4 {
        assert_eq!(run(&t.db.now(), &q).len(), 1);
    }
    // inside `with`, on the writer
    let n =
        t.db.with(
            |tx| {
                tx.assert(v("c"), v("n"), s("Alpha"), Valid::ALWAYS)
                    .map(|_| ())
            },
            |view| Ok(view.execute_ir(&q, &Params::new())?.len()),
        )
        .unwrap();
    assert_eq!(n, 2);
    let _ = Op::unit();
}
