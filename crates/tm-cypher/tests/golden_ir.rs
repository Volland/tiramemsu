//! Golden IR snapshots of the pattern lowering (design Decision 5). A recording
//! runner returns no rows and keeps the text of every IR query the program issues.

use tm_core::{Result, Tx, Value};
use tm_cypher::{compile, CompileCtx, CypherParams, Rows, Runner, Vocab};
use tm_ir::{IrQuery, Params};

#[derive(Default)]
struct Recorder {
    queries: Vec<String>,
}

impl Runner for Recorder {
    fn run_ir(&mut self, q: &IrQuery, _p: &Params) -> Result<Rows> {
        self.queries.push(q.to_string());
        Ok(Rows::default())
    }
    fn now_ms(&self) -> i64 {
        0
    }
    fn object_id(&mut self, _v: &Value) -> Result<Option<i64>> {
        Ok(None)
    }
    fn volatile_of(&mut self, _s: &Value) -> Result<Vec<(String, Value)>> {
        Ok(vec![])
    }
    fn writable(&self) -> bool {
        false
    }
    fn with_tx(&mut self, _f: &mut dyn FnMut(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        unreachable!()
    }
}

/// The IR of the pattern queries of `text` (the schema-flag read is dropped).
fn ir_of(text: &str) -> String {
    let ctx = CompileCtx {
        vocab: Vocab::default(),
        view: tm_ir::View::NOW,
        writable: false,
    };
    let params = CypherParams::new();
    let prog = compile(text, &params, &ctx).unwrap();
    let mut r = Recorder::default();
    tm_cypher::exec::run(&prog, &params, &mut r).unwrap();
    r.queries
        .into_iter()
        .filter(|q| !q.contains("sys:isEdge"))
        .collect::<Vec<_>>()
        .join("\n---\n")
}

// cypher-read "Label matches rdf:type": the label generator is a distinct projection
#[test]
fn label_generator() {
    insta::assert_snapshot!(ir_of("MATCH (n:Person) RETURN n"));
}

// "Multiple labels are conjunctive" / "Label disjunction"
#[test]
fn extra_labels_and_disjunction() {
    insta::assert_snapshot!(
        "conjunctive_labels",
        ir_of("MATCH (n:Person:Employee) RETURN n")
    );
    insta::assert_snapshot!(
        "label_disjunction",
        ir_of("MATCH (n:Person|Company) RETURN n")
    );
}

// "Unlabelled node scan excludes statements, transactions and class IRIs"
#[test]
fn bare_node_scan() {
    insta::assert_snapshot!(ir_of("MATCH (n) RETURN n"));
}

// "Inline property map filters": existence tests, not row-multiplying joins
#[test]
fn property_map_is_an_existence_test() {
    insta::assert_snapshot!(ir_of(
        "MATCH (n:Person {name: 'Alice', score: 30.0}) RETURN n"
    ));
}

// "Directed match" / "Type disjunction" / "Undirected match returns both orientations"
#[test]
fn relationship_patterns() {
    insta::assert_snapshot!("directed", ir_of("MATCH (a)-[r:worksAt]->(b) RETURN r"));
    insta::assert_snapshot!("reversed", ir_of("MATCH (a)<-[r:worksAt]-(b) RETURN r"));
    insta::assert_snapshot!("undirected", ir_of("MATCH (a)-[r:knows]-(b) RETURN r"));
    insta::assert_snapshot!(
        "type_disjunction",
        ir_of("MATCH (a)-[r:knows|worksAt]->(b) RETURN r")
    );
}

// "Untyped pattern hides sys predicates" and "rdf:type is never a relationship"
#[test]
fn untyped_relationship_hides_sys_and_rdf_type() {
    insta::assert_snapshot!(ir_of("MATCH (a)-[r]->(b) RETURN r"));
}

// "Same relationship not reused within a clause": pairwise eid inequality; and the opt-out
#[test]
fn isomorphism_filters() {
    insta::assert_snapshot!(
        "default_mode",
        ir_of("MATCH (x)-[r1:knows]->(y)-[r2:knows]->(z) RETURN x")
    );
    insta::assert_snapshot!(
        "repeatable_elements",
        ir_of("MATCH REPEATABLE ELEMENTS (x)-[r1:knows]->(y)-[r2:knows]->(z) RETURN x")
    );
}

// dual view: a relationship variable in node position is the same IR variable
#[test]
fn dual_view_variable_is_shared() {
    insta::assert_snapshot!(ir_of(
        "MATCH (a)-[r:WORKS_AT]->(c), (b:Belief)-[:SUPPORTED_BY]->(r) RETURN a"
    ));
}

// temporal scope: the view of a `USE` clause reaches every pattern of its scope
#[test]
fn use_clause_sets_the_pattern_view() {
    insta::assert_snapshot!(
        "as_of",
        ir_of("USE AS OF 150 MATCH (a {`@id`: 'v:alice'})-[:worksAt]->(before) RETURN before")
    );
    insta::assert_snapshot!(
        "history_valid_at",
        ir_of("USE HISTORY VALID AT date('2026-03-01') MATCH (a)-[:worksAt]->(b) RETURN b")
    );
}
