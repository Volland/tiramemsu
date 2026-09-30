//! Bitemporal memory: a fact with a valid interval, a correction, and four ways to look back.
//! Run with `cargo run -p tiramemsu --example time_travel`.
use std::sync::Arc;

use tiramemsu::*;

fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// The sorted `?o` values of Alice's employer as seen through `view`.
fn employers(view: &View<'_>) -> Result<Vec<String>> {
    let r = view.sparql("SELECT ?o WHERE { v:alice v:worksAt ?o }")?;
    let sol = r.solutions().expect("a SELECT gives solutions");
    let mut out: Vec<String> = (0..sol.rows.len())
        .filter_map(|i| sol.get(i, "o"))
        .map(|o| o.to_string())
        .collect();
    out.sort();
    Ok(out)
}

fn main() -> Result<()> {
    let dir = std::env::temp_dir().join("tiramemsu-time-travel");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    // A manual clock makes transaction instants, and so this output, deterministic.
    let clock = Arc::new(ManualClock::new(1_000_000));
    let db = Db::open(
        dir.join("memory.db"),
        OpenOptions {
            clock: clock.clone(),
            ..OpenOptions::default()
        },
    )?;

    // tx 1: Alice worked at Acme from valid time 100 to 200 (epoch ms, half open).
    let mut fact = None;
    let t1 = db.transact(TxOptions::default(), |tx| {
        fact = Some(
            tx.assert(
                v("alice"),
                v("worksAt"),
                v("acme"),
                Valid::between(100, 200),
            )?
            .eid(),
        );
        Ok(())
    })?;

    // tx 2: we learn she actually stayed until 300. A correction, not an overwrite.
    clock.advance(1_000);
    let t2 = db.transact(TxOptions::default(), |tx| {
        let patch = Patch {
            v_to: Some(Some(300)),
            ..Patch::default()
        };
        tx.supersede(fact.expect("asserted"), patch)?;
        Ok(())
    })?;
    println!("committed tx {} then tx {}", t1.t.0, t2.t.0);

    println!(
        "\nas_of(tx 1) valid_at(250): {:?}",
        employers(&db.as_of(TimeRef::Tx(t1.t.0)).valid_at(250))?
    );
    println!(
        "now         valid_at(250): {:?}",
        employers(&db.now().valid_at(250))?
    );
    println!(
        "now         valid_at(300): {:?}  (half open: 300 is outside)",
        employers(&db.now().valid_at(300))?
    );
    println!(
        "as_of(instant of tx 1):      {:?}",
        employers(&db.as_of(TimeRef::Instant(t1.instant)))?
    );

    // History keeps every row with its real lifetime.
    println!("\nhistory (predicate, valid, believed from tx, retracted in tx, why):");
    let history = db.history();
    for t in history.triples(None, None, None)? {
        println!(
            "  {:<32} valid [{:?}, {:?})  t_add={}  t_ret={:?}  {:?}",
            history.decode(t.p)?.to_string(),
            t.v_from,
            t.v_to,
            t.t_add.0,
            t.t_ret.map(|t| t.0),
            t.ret_kind
        );
    }
    Ok(())
}
