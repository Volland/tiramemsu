//! The storage contract on the in-memory reference store.

use std::sync::Arc;

use tm_store::contract;
use tm_store::mem::MemStore;

// @tg: verifies:: [[openspec:storage-trait#Typed storage operations#Scan by bound positions]] {since: "2026-10-27", level: unit}
#[test]
fn scan_by_bound_positions() {
    contract::scan_by_bound_positions(&MemStore::new());
}

// @tg: verifies:: [[openspec:storage-trait#Typed storage operations#Scan in eid order]] {since: "2026-10-27", level: unit}
#[test]
fn scan_in_eid_order() {
    contract::scan_in_eid_order(&MemStore::new());
}

// @tg: verifies:: [[openspec:storage-trait#Typed storage operations#Live and all scopes]] {since: "2026-10-27", level: unit}
#[test]
fn live_and_all_scopes() {
    contract::live_and_all_scopes(&MemStore::new());
}

// @tg: verifies:: [[openspec:storage-trait#Never forget by construction#Second retraction is refused]] {since: "2026-10-27", level: unit}
#[test]
fn second_retraction_is_refused() {
    contract::second_retraction_is_refused(&MemStore::new());
}

// @tg: verifies:: [[openspec:storage-trait#Never forget by construction#Duplicate key is refused]] {since: "2026-10-27", level: unit}
#[test]
fn duplicate_key_is_refused() {
    contract::duplicate_key_is_refused(&MemStore::new());
}

// @tg: verifies:: [[openspec:storage-trait#One engine for every backend#Assert on the reference store]] {since: "2026-10-27", level: unit}
#[test]
fn assert_on_the_reference_store() {
    contract::assert_on_the_store(Arc::new(MemStore::new()));
}

// @tg: verifies:: [[openspec:storage-trait#One engine for every backend#Cascade on the reference store]] {since: "2026-10-27", level: unit}
#[test]
fn cascade_on_the_reference_store() {
    contract::cascade_on_the_store(Arc::new(MemStore::new()));
}

// @tg: verifies:: [[openspec:storage-trait#Speculation without nested savepoints#Dry run burns ids]] {since: "2026-10-27", level: unit}
#[test]
fn dry_run_burns_ids() {
    contract::dry_run_burns_ids(Arc::new(MemStore::new()));
}

// @tg: verifies:: [[openspec:storage-trait#Snapshot reads#Reader ignores a later commit]] {since: "2026-10-27", level: unit}
#[test]
fn reader_ignores_a_later_commit() {
    contract::reader_ignores_a_later_commit(Arc::new(MemStore::new()));
}
