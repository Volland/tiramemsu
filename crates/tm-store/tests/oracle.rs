//! The SQLite engine is the oracle for the trait engine on `MemStore`
//! (design D-6 of `add-store-trait`).

use std::sync::Arc;

use tm_store::mem::MemStore;

#[path = "common/oracle.rs"]
mod oracle;

// @tg: verifies:: [[openspec:storage-trait#Engine equivalence with SQLite#Random histories agree]] {since: "2026-10-27", level: integration}
#[test]
fn random_histories_agree() {
    oracle::random_histories(|| Arc::new(MemStore::new()));
}

#[test]
fn identical_on_a_fixed_history() {
    oracle::fixed_history(Arc::new(MemStore::new()));
}
