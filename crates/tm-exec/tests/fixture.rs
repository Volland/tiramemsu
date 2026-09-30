//! Sanity checks of the shared fixtures.

mod common;

use common::*;

#[test]
fn layer_fixture_exposes_eids() {
    let f = layers();
    let e1 = f.eid("e1");
    let now = f.db().now();
    let about_e1 = now.triples(Some(e1.oid()), None, None).unwrap();
    assert_eq!(about_e1.len(), 2, "confidence and confirmedBy");
    let e7 = f.eid("e7");
    let t = now.triples(Some(e7.oid()), None, None).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(now.triples(None, None, Some(e1.oid())).unwrap()[0].eid, e7);
    assert_eq!(f.t.last_t(), 2);
    for n in ["e1", "e2", "e3", "e7", "e8"] {
        assert!(f.eids.contains_key(n));
    }
}
