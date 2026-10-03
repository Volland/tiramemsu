//! Shared by the native and the wasm32 tests of the WASM host: one fixture
//! history, written by either host and checked through the other, and a dump of
//! every engine table that both hosts must agree on byte for byte.

#![allow(dead_code)]

use tiramemsu::*;

pub fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

pub fn lit(s: &str) -> Value {
    Value::str(s)
}

/// Three transactions: plain statements, typed and language literals, a tx
/// annotation, a fresh node, then a retraction (history the now-view hides).
pub fn write_fixture(db: &Db) -> Vec<TxReport> {
    let mut out = Vec::new();
    out.push(
        db.transact(TxOptions::default(), |tx| {
            tx.assert(v("alice"), v("knows"), v("bob"), Valid::ALWAYS)?;
            tx.assert(v("alice"), v("name"), lit("Alice"), Valid::ALWAYS)?;
            tx.assert(
                v("alice"),
                v("age"),
                Value::literal("42", Some("http://www.w3.org/2001/XMLSchema#integer"), None),
                Valid::ALWAYS,
            )?;
            tx.assert(
                v("bob"),
                v("note"),
                Value::literal("Tiramisu mit Schichten", None, Some("de")),
                Valid::ALWAYS,
            )?;
            tx.meta(v("source"), lit("worker"))?;
            Ok(())
        })
        .unwrap(),
    );
    out.push(
        db.transact(TxOptions::default(), |tx| {
            let n = tx.new_node()?;
            tx.assert(n, v("label"), lit("anonymous layer"), Valid::ALWAYS)?;
            tx.assert(v("bob"), v("knows"), v("carol"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap(),
    );
    let knows = out[0].asserted[0];
    out.push(
        db.transact(TxOptions::default(), |tx| {
            assert!(tx.retract(knows)?);
            Ok(())
        })
        .unwrap(),
    );
    out
}

/// The fixture is visible as written: current and historical statements, tx count.
pub fn check_fixture(db: &Db) {
    let now = db.now().triples(None, None, None).unwrap();
    let history = db.history().triples(None, None, None).unwrap();
    // 7 live user statements (alice knows bob is retracted) plus engine statements
    assert!(
        history.len() > now.len(),
        "history must keep the retraction"
    );
    let names = db
        .read_sql("SELECT count(*) FROM tx")
        .unwrap()
        .remove(0)
        .remove(0);
    assert_eq!(names, SqlValue::Integer(3), "three committed transactions");
    let alice = db.now().encode(&v("alice")).unwrap().expect("interned");
    let knows = db.now().encode(&v("knows")).unwrap().expect("interned");
    assert!(db
        .now()
        .triples(Some(alice), Some(knows), None)
        .unwrap()
        .is_empty());
    assert_eq!(
        db.history()
            .triples(Some(alice), Some(knows), None)
            .unwrap()
            .len(),
        1
    );
}

/// Every row of the engine tables and the id counters, in a fixed order.
pub fn dump(db: &Db) -> Vec<String> {
    let mut out = Vec::new();
    for (table, order) in [
        ("term", "1"),
        ("tx", "1"),
        ("triple", "1"),
        ("volatile", "1, 2"),
        ("pred_multi", "1"),
    ] {
        for row in db
            .read_sql(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .unwrap()
        {
            out.push(format!("{table}: {row:?}"));
        }
    }
    for row in db
        .read_sql(
            "SELECT key, value FROM meta WHERE key IN ('format_version', 'next_term', \
             'next_node', 'next_bnode', 'next_stmt', 'last_t', 'last_instant', \
             'multi_version') ORDER BY key",
        )
        .unwrap()
    {
        out.push(format!("meta: {row:?}"));
    }
    out
}

/// The largest statement number in the file.
pub fn max_stmt(db: &Db) -> i64 {
    db.read_sql("SELECT value FROM meta WHERE key = 'next_stmt'")
        .unwrap()
        .remove(0)
        .remove(0)
        .as_i64()
        .unwrap()
        - 1
}

/// One more transaction: its number follows `last_t` and its statement numbers
/// and node id follow every id allocated before (no id is reused).
pub fn continue_history(db: &Db, last_t: u64) {
    let before = max_stmt(db);
    let node_before = db
        .read_sql("SELECT value FROM meta WHERE key = 'next_node'")
        .unwrap()
        .remove(0)
        .remove(0)
        .as_i64()
        .unwrap();
    let r = db
        .transact(TxOptions::default(), |tx| {
            let n = tx.new_node()?;
            tx.assert(n, v("label"), lit("after the handoff"), Valid::ALWAYS)?;
            tx.assert(v("carol"), v("knows"), v("alice"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(r.t.t(), last_t + 1, "tx numbering continues");
    assert!(r.asserted.iter().all(|e| e.n() as i64 > before));
    let node_after = db
        .read_sql("SELECT value FROM meta WHERE key = 'next_node'")
        .unwrap()
        .remove(0)
        .remove(0)
        .as_i64()
        .unwrap();
    assert_eq!(node_after, node_before + 1);
}

/// Open options with the query engine and the LFTJ operator, so every native
/// table function is installed.
pub fn engine_options() -> OpenOptions {
    let mut opts = OpenOptions {
        clock: std::sync::Arc::new(tm_wasm::WasmClock),
        text_index: true,
        ..OpenOptions::default()
    };
    opts.planner.lftj.enabled = true;
    opts
}

/// Core-tier open options: no query engine.
pub fn core_options() -> OpenOptions {
    OpenOptions {
        clock: std::sync::Arc::new(tm_wasm::WasmClock),
        query_engine: false,
        ..OpenOptions::default()
    }
}

/// Every SQL function and native table function the query engine needs is
/// registered on the connection that runs queries (the writer: the WASM host
/// has no readers), and SPARQL, Cypher, a path and text recall execute on it.
pub fn check_operators(db: &Db) {
    assert_eq!(db.reader_count(), 0);
    let functions: Vec<String> = db
        .read_sql("SELECT DISTINCT name FROM pragma_function_list")
        .unwrap()
        .into_iter()
        .filter_map(|r| r[0].as_str().map(str::to_string))
        .collect();
    let required = tm_exec::udf::scalar_functions()
        .into_iter()
        .map(|f| f.name)
        .chain(
            tm_exec::udf::aggregate_functions()
                .into_iter()
                .map(|f| f.name),
        );
    for name in required {
        assert!(functions.contains(&name), "function {name} not registered");
    }
    let modules: Vec<String> = db
        .read_sql("SELECT name FROM pragma_module_list")
        .unwrap()
        .into_iter()
        .filter_map(|r| r[0].as_str().map(str::to_string))
        .collect();
    for name in [
        tm_exec::path::vtab::NAME,
        tm_exec::text::NAME,
        tm_exec::lftj::NAME,
        "rarray",
    ] {
        assert!(
            modules.iter().any(|m| m == name),
            "table function {name} not registered"
        );
    }
    let sparql = db
        .now()
        .sparql("SELECT ?o WHERE { <urn:tiramemsu:v:bob> <urn:tiramemsu:v:knows> ?o }")
        .unwrap();
    assert_eq!(sparql.solutions().expect("select").rows.len(), 1);
    // a property path runs on the native path operator (tm_path)
    let reach = db
        .now()
        .sparql("SELECT ?o WHERE { <urn:tiramemsu:v:bob> <urn:tiramemsu:v:knows>+ ?o }")
        .unwrap();
    assert_eq!(reach.solutions().expect("select").rows.len(), 1);
    let cypher = db
        .now()
        .cypher(
            "MATCH (a)-[r]->(b) RETURN count(*) AS n",
            &CypherParams::default(),
        )
        .unwrap();
    assert_eq!(cypher.rows.len(), 1);
    let hits = db.now().text_search(&TextQuery::new("schichten")).unwrap();
    assert_eq!(hits.len(), 1, "text recall through tm_text");
}
