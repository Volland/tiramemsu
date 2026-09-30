//! Layers: a belief supported by a fact that carries confidence and source, queried with
//! RDF 1.2 annotations and walked with a path that crosses layers.
//! Run with `cargo run -p tiramemsu --example layers_and_paths`.
use tiramemsu::*;

fn v(local: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{local}"))
}

fn main() -> Result<()> {
    let dir = std::env::temp_dir().join("tiramemsu-layers-and-paths");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = Db::open(dir.join("memory.db"), OpenOptions::default())?;

    // Layer 0: a fact. Layer 1: its confidence and source. Layer 2: a belief resting on it.
    db.transact(TxOptions::default(), |tx| {
        let fact = tx
            .assert(v("alice"), v("worksAt"), v("acme"), Valid::ALWAYS)?
            .eid();
        let conf = Value::literal("0.8", Some("http://www.w3.org/2001/XMLSchema#double"), None);
        tx.assert(Value::Stmt(fact), v("confidence"), &conf, Valid::ALWAYS)?;
        tx.assert(
            Value::Stmt(fact),
            v("source"),
            Value::str("chat-2026-09-29"),
            Valid::ALWAYS,
        )?;
        tx.assert(
            v("belief9"),
            v("supportedBy"),
            Value::Stmt(fact),
            Valid::ALWAYS,
        )?;
        tx.assert(
            v("belief9"),
            v("claim"),
            Value::str("alice is at acme"),
            Valid::ALWAYS,
        )?;
        Ok(())
    })?;
    let view = db.now();

    // SPARQL: RDF 1.2 annotation syntax reads the layer on a fact.
    let r = view.sparql(
        "SELECT ?c ?s WHERE { v:alice v:worksAt v:acme {| v:confidence ?c ; v:source ?s |} }",
    )?;
    let sol = r.solutions().expect("solutions");
    println!("annotation query:");
    for i in 0..sol.rows.len() {
        println!(
            "  confidence={} source={}",
            sol.get(i, "c").expect("c"),
            sol.get(i, "s").expect("s")
        );
    }

    // SPARQL: which beliefs rest on which facts, and how sure are we?
    let r = view.sparql("SELECT ?b ?c WHERE { ?b v:supportedBy ?e . ?e v:confidence ?c }")?;
    let sol = r.solutions().expect("solutions");
    println!("beliefs and the confidence of their support:");
    for i in 0..sol.rows.len() {
        println!(
            "  {} <- {}",
            sol.get(i, "b").expect("b"),
            sol.get(i, "c").expect("c")
        );
    }

    // Cypher sees the same layers: a relationship property and a :Statement node.
    let r = view.cypher(
        "MATCH (p)-[r:worksAt]->(o) RETURN p, o, r.confidence AS confidence",
        &CypherParams::default(),
    )?;
    println!("cypher columns {:?}, {} row(s)", r.columns, r.rows.len());

    // A path from the belief, through the fact it rests on, to that fact's endpoints.
    // `sys:subject` and `sys:object` are the virtual hops from a fact id to its ends.
    let start = view.encode(&v("belief9"))?.expect("belief9 is stored");
    let rows = view.path(
        start,
        "supportedBy/(sys:subject|sys:object)",
        PathMode::Reachability,
        u32::MAX,
    )?;
    let mut ends = Vec::new();
    for row in &rows {
        ends.push(view.decode(row.end)?.to_string());
    }
    ends.sort();
    println!("path belief9 -> supportedBy -> (subject|object): {ends:?}");
    Ok(())
}
