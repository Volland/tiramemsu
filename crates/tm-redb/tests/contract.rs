//! The storage contract and the SQLite oracle on redb.

use std::sync::Arc;

use tm_redb::{RedbOptions, RedbStore};
use tm_store::{contract, Backend};

#[path = "../../tm-store/tests/common/oracle.rs"]
mod oracle;

/// A new redb store in its own temporary directory, kept alive with the store.
struct Fresh {
    store: RedbStore,
    _dir: tempfile::TempDir,
}

impl Backend for Fresh {
    fn name(&self) -> &'static str {
        self.store.name()
    }
    fn read(&self) -> tm_core::Result<Box<dyn tm_store::ReadStore + Send>> {
        self.store.read()
    }
    fn write(&self) -> tm_core::Result<Box<dyn tm_store::WriteTxn>> {
        self.store.write()
    }
}

fn fresh() -> Fresh {
    let dir = tempfile::tempdir().unwrap();
    let store = RedbStore::create(&dir.path().join("t.redb"), RedbOptions::default()).unwrap();
    Fresh { store, _dir: dir }
}

#[test]
fn scan_by_bound_positions() {
    contract::scan_by_bound_positions(&fresh());
}

#[test]
fn scan_in_eid_order() {
    contract::scan_in_eid_order(&fresh());
}

#[test]
fn live_and_all_scopes() {
    contract::live_and_all_scopes(&fresh());
}

#[test]
fn second_retraction_is_refused() {
    contract::second_retraction_is_refused(&fresh());
}

#[test]
fn duplicate_key_is_refused() {
    contract::duplicate_key_is_refused(&fresh());
}

#[test]
fn assert_on_redb() {
    contract::assert_on_the_store(Arc::new(fresh()));
}

#[test]
fn cascade_on_redb() {
    contract::cascade_on_the_store(Arc::new(fresh()));
}

#[test]
fn dry_run_burns_ids() {
    contract::dry_run_burns_ids(Arc::new(fresh()));
}

#[test]
fn reader_ignores_a_later_commit() {
    contract::reader_ignores_a_later_commit(Arc::new(fresh()));
}

#[test]
fn oracle_fixed_history() {
    oracle::fixed_history(Arc::new(fresh()));
}

// SQLite and redb agree on derive, re-derive, derived keys, supersede and the support cascade
// @tg: verifies:: [[openspec:derived-beliefs#Derivation in both engines#Oracle agreement]] {since: "2026-10-28", level: integration}
#[test]
fn oracle_derived_history() {
    oracle::derived_history(Arc::new(fresh()));
}

#[test]
fn oracle_random_histories() {
    oracle::random_histories(|| Arc::new(fresh()));
}
