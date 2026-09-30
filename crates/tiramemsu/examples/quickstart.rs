//! The README quick start: a fact with layers, a correction, time travel, both query languages.
//! Run with `cargo run -p tiramemsu --example quickstart`.
use tiramemsu::*;

fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

fn main() -> Result<()> {
    let dir = std::env::temp_dir().join("tiramemsu-quickstart");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open(dir.join("memory.db"), OpenOptions::default())?;

    // 1. A fact is a statement with its own id (an eid), so it can carry layers.
    let mut fact = None;
    db.transact(TxOptions::default(), |tx| {
        let eid = match tx.assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)? {
            Asserted::New(e) | Asserted::Existing(e) => e,
        };
        fact = Some(eid);
        let conf = Value::literal("0.8", Some("http://www.w3.org/2001/XMLSchema#double"), None);
        tx.assert(Value::Stmt(eid), v("confidence"), &conf, Valid::ALWAYS)?;
        tx.assert(
            Value::Stmt(eid),
            v("source"),
            Value::str("chat-2026-09-29"),
            Valid::ALWAYS,
        )?;
        tx.meta(v("author"), v("agent7"))?; // metadata on the transaction itself
        Ok(())
    })?;

    // 2. A correction replays the fact and its layers under new ids. Nothing is deleted.
    // Alice actually works at Globex; the confidence and source layers follow the correction.
    let corrected = db.transact(TxOptions::default(), |tx| {
        let patch = Patch {
            o: Some(v("globex")),
            ..Patch::default()
        };
        tx.supersede(fact.unwrap(), patch)?;
        Ok(())
    })?;

    // 3. Ask what is believed now, and what was believed before the correction.
    let sparql = "SELECT ?who ?org WHERE { ?who v:worksAt ?org }";
    let before = db.as_of(TimeRef::Tx(corrected.t.0 - 1)).sparql(sparql)?;
    let after = db.now().sparql(sparql)?;
    println!("before the correction: {before:?}");
    println!("after:                 {after:?}");

    // 4. The same store in Cypher: the confidence layer reads as a relationship property.
    let cypher = "MATCH (p)-[r:worksAt]->(o) RETURN p, o, r.confidence";
    println!(
        "cypher: {:?}",
        db.now().cypher(cypher, &CypherParams::default())?
    );
    Ok(())
}
