//! Spec `view-scoped-scans` "One snapshot per query".

mod common;

use std::sync::Arc;

use common::*;
use tiramemsu::ir::builder::IrBuilder;
use tiramemsu::*;

// "Concurrent commit during a query": a writer commits between planning and
// decoding (through the test hook in `exec.rs`); the result is exactly the state
// before that transaction.
#[test]
fn concurrent_commit_is_invisible_to_a_running_query() {
    let t = Arc::new(TestDb::new());
    let mut e1 = None;
    t.tx(|tx| {
        e1 = Some(
            tx.assert(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?
                .eid(),
        );
        Ok(())
    });
    let b = IrBuilder::sparql();
    let q = b.query(b.triple("v:alice", "v:knows", "?o"));
    let t2 = t.clone();
    let e1 = e1.unwrap();
    t.db.set_query_hook(Some(Arc::new(move || {
        t2.tx(|tx| {
            tx.retract(e1)?;
            tx.assert(v("alice"), v("knows"), v("carol"), Valid::ALWAYS)?;
            Ok(())
        });
    })));
    let during = run(&t.db.now(), &q);
    t.db.set_query_hook(None);
    assert_eq!(
        rows(&during),
        expect(&[&["bob"]]),
        "the before-state, not a mix"
    );
    let after = run(&t.db.now(), &q);
    assert_eq!(rows(&after), expect(&[&["carol"]]));
}
