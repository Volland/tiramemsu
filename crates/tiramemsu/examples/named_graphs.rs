//! Named graphs as sessions: statements join graphs by membership, `GRAPH` queries select
//! them, and leaving a session removes only the membership.
//! Run with `cargo run -p tiramemsu --example named_graphs`.
use tiramemsu::*;

fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

/// The sorted subjects of `?who` in the solutions of `query`.
fn who(db: &Db, query: &str) -> Result<Vec<String>> {
    let r = db.now().sparql(query)?;
    let sol = r.solutions().expect("solutions");
    let mut out: Vec<String> = (0..sol.rows.len())
        .filter_map(|i| sol.get(i, "who"))
        .map(|w| w.to_string())
        .collect();
    out.sort();
    Ok(out)
}

fn main() -> Result<()> {
    let dir = std::env::temp_dir().join("tiramemsu-named-graphs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = Db::open(dir.join("memory.db"), OpenOptions::default())?;

    // Two sessions. The Rust API adds an existing fact to a graph; SPARQL does it in one go.
    let session1 = v("session1");
    let mut alice_at_acme = None;
    db.transact(TxOptions::default(), |tx| {
        let e = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        tx.add_to_graph(e, &session1, AssertOpts::default())?;
        alice_at_acme = Some(e);
        Ok(())
    })?;
    db.now().sparql(
        "INSERT DATA { GRAPH v:session2 { v:bob v:worksAt v:acme . v:alice v:worksAt v:acme } }",
    )?;

    let all = "SELECT ?who WHERE { ?who v:worksAt ?o }";
    let s1 = "SELECT ?who WHERE { GRAPH v:session1 { ?who v:worksAt ?o } }";
    let s2 = "SELECT ?who WHERE { GRAPH v:session2 { ?who v:worksAt ?o } }";
    println!("all statements:  {:?}", who(&db, all)?);
    println!("in session1:     {:?}", who(&db, s1)?);
    println!("in session2:     {:?}", who(&db, s2)?);
    let view = db.now();
    let mut graphs = view.graphs()?;
    graphs.dedup();
    for g in graphs {
        println!(
            "graph {:?}: {} member(s)",
            view.decode(g)?.to_string(),
            view.graph_members(g)?.len()
        );
    }

    // The same fact is a member of both sessions (one statement, two memberships). Leaving
    // session1 retracts the membership only. The fact and its session2 membership stay.
    let report = db.transact(TxOptions::default(), |tx| {
        let removed = tx.remove_from_graph(alice_at_acme.expect("asserted"), &session1)?;
        println!("\nremoved from session1: {removed}");
        Ok(())
    })?;
    println!(
        "memberships retracted: {}, statements retracted: {}",
        report.memberships_retracted.len(),
        report.retracted.len()
    );
    println!("in session1:     {:?}", who(&db, s1)?);
    println!("in session2:     {:?}", who(&db, s2)?);
    println!("all statements:  {:?}", who(&db, all)?);

    // The past is intact: session1 still had Alice before the change.
    let before = db.as_of(TimeRef::Tx(1)).sparql(s1)?;
    println!(
        "session1 as of tx 1: {} row(s)",
        before.solutions().expect("solutions").rows.len()
    );
    Ok(())
}
