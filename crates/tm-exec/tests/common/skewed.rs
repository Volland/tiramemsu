//! The skewed plan fixture (`lat.md/tests#Query#Skewed Joins Start Selective`),
//! loaded through the ordinary API only and never analysed explicitly:
//!
//! - 2 000 nodes; 90 % are `v:Person`, 10 % are `v:Org` (`v:type`);
//! - a high-fanout `v:knows` predicate (about 18 000 edges among persons);
//! - a 50-row predicate `v:rare` (from 50 persons to `v:Special`);
//! - `v:works` (person to org), and churned `v:status` properties that were
//!   asserted, retracted and asserted again.
//!
//! One big first commit crosses `optimize_every`, so M0's automatic statistics
//! appear without a call to `optimize()`.

use tiramemsu::*;

use super::{v, TestDb};

pub const PERSONS: usize = 1800;
pub const ORGS: usize = 200;
pub const RARE: usize = 50;

pub fn skewed() -> TestDb {
    let t = TestDb::new();
    t.tx(|tx| {
        for i in 0..PERSONS {
            tx.assert(v(&format!("p{i}")), v("type"), v("Person"), Valid::ALWAYS)?;
            tx.assert(
                v(&format!("p{i}")),
                v("works"),
                v(&format!("o{}", i % ORGS)),
                Valid::ALWAYS,
            )?;
            for k in 1..=10 {
                let j = (i * 7 + k * 13) % PERSONS;
                if j != i {
                    tx.assert(
                        v(&format!("p{i}")),
                        v("knows"),
                        v(&format!("p{j}")),
                        Valid::ALWAYS,
                    )?;
                }
            }
        }
        for i in 0..ORGS {
            tx.assert(v(&format!("o{i}")), v("type"), v("Org"), Valid::ALWAYS)?;
        }
        for i in 0..RARE {
            tx.assert(
                v(&format!("p{}", i * 31)),
                v("rare"),
                v("Special"),
                Valid::ALWAYS,
            )?;
        }
        // churn: assert, retract, re-assert
        let mut first = Vec::new();
        for i in 0..400 {
            let e = tx.assert(v(&format!("p{i}")), v("status"), v("active"), Valid::ALWAYS)?;
            first.push(e.eid());
        }
        for e in first {
            tx.retract(e)?;
        }
        for i in 0..400 {
            tx.assert(v(&format!("p{i}")), v("status"), v("away"), Valid::ALWAYS)?;
        }
        Ok(())
    });
    t
}

/// The number of statistics rows the planner has (SQLite's `sqlite_stat1`).
pub fn stat_rows(t: &TestDb) -> i64 {
    t.db.read_sql("SELECT count(*) FROM sqlite_stat1")
        .map(|r| r[0][0].as_i64().unwrap_or(0))
        .unwrap_or(0)
}
