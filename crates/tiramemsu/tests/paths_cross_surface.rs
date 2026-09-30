//! Task 12.2: `View::path`, `tm_path`, SPARQL `+` and Cypher trails agree on endpoint
//! sets and hop counts over random fixtures.
mod cypher_common;

use std::collections::BTreeMap;

use cypher_common::*;
use proptest::prelude::*;
use tiramemsu::*;

fn local(v: &Value) -> String {
    match v {
        Value::Iri(i) => i.trim_start_matches("urn:tiramemsu:v:").to_string(),
        other => format!("{other:?}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn surfaces_agree(
        edges in prop::collection::vec((0usize..4, 0usize..4), 1..8),
        start in 0usize..4,
    ) {
        let t = T::new();
        let name = |i: usize| format!("n{i}");
        t.tx(|tx| {
            for i in 0..4 {
                tx.assert(v(&name(i)), v("id"), sv(&name(i)), Valid::ALWAYS)?;
            }
            for (a, b) in &edges {
                tx.create(v(&name(*a)), v("knows"), v(&name(*b)), Valid::ALWAYS)?;
            }
            Ok(())
        });
        let view = t.db.now();
        let s = view.encode(&v(&name(start))).unwrap().unwrap();
        // the API
        let api: BTreeMap<String, u32> = view
            .path(s, "knows+", PathMode::Reachability, u32::MAX)
            .unwrap()
            .iter()
            .map(|r| (local(&view.decode(r.end).unwrap()), r.hops))
            .collect();
        // tm_path
        let sql = t.db.read_sql(&format!(
            "SELECT \"end\", hops FROM tm_path({}, 'knows+', 'REACH')", s.raw()
        )).unwrap();
        let via_sql: BTreeMap<String, u32> = sql
            .iter()
            .map(|r| {
                let id = ObjectId::from_raw(r[0].as_i64().unwrap());
                (local(&view.decode(id).unwrap()), r[1].as_i64().unwrap() as u32)
            })
            .collect();
        prop_assert_eq!(&api, &via_sql);
        // SPARQL: the endpoint set
        let SparqlResult::Solutions(sol) = view
            .sparql(&format!("SELECT ?y WHERE {{ v:{} v:knows+ ?y }}", name(start)))
            .unwrap()
        else { panic!() };
        let mut sparql: Vec<String> = sol.rows.iter().map(|r| local(r[0].as_ref().unwrap())).collect();
        sparql.sort();
        prop_assert_eq!(sparql, api.keys().cloned().collect::<Vec<_>>());
        // Cypher: distinct trail ends and their shortest lengths
        let r = view
            .cypher(
                &format!(
                    "MATCH p = (x {{id:'{}'}})-[:knows*]->(y) RETURN y.id AS y, min(length(p)) AS h",
                    name(start)
                ),
                &no_params(),
            )
            .unwrap();
        let cy: BTreeMap<String, u32> = r
            .rows
            .iter()
            .map(|row| {
                let CypherValue::String(y) = &row[0] else { panic!() };
                let CypherValue::Integer(h) = &row[1] else { panic!() };
                (y.clone(), *h as u32)
            })
            .collect();
        prop_assert_eq!(cy, api);
    }
}
