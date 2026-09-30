//! Runs the SPARQL half of the differential corpus (task 11.4). The Cypher half
//! and the `@lat` reference for `tests#Query#Differential SPARQL Cypher` live in
//! the harness of `add-cypher-frontend`.
mod differential;
mod sparql_common;

use differential::*;
use sparql_common::*;

#[test]
fn sparql_corpus_returns_the_expected_rows() {
    let t = T::new();
    fixture(&t.db);
    for case in CASES {
        let got = rows(&t.sel(case.sparql));
        let want: std::collections::BTreeSet<Vec<String>> = case
            .rows
            .iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect();
        assert_eq!(got, want, "{}: {}", case.name, case.sparql);
    }
}
