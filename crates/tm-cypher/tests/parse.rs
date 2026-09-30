//! Parser adapter, extension pre-pass and unsupported-feature tests
//! (specs `cypher-read` parse and unsupported requirements, `cypher-temporal-clauses`).

use proptest::prelude::*;
use tm_cypher::ast::*;
use tm_cypher::parse::adapter::parse;
use tm_cypher::parse::prepass;
use tm_cypher::{CypherError, Span};

fn unsupported(q: &str) -> String {
    match parse(q) {
        Err(CypherError::Unsupported { feature, .. }) => feature,
        other => panic!("expected Unsupported for {q}, got {other:?}"),
    }
}

fn parse_span(q: &str) -> Span {
    match parse(q) {
        Err(CypherError::Parse { span, .. }) => span,
        other => panic!("expected Parse error for {q}, got {other:?}"),
    }
}

fn clauses(q: &str) -> Vec<Clause> {
    parse(q).unwrap().parts.remove(0).clauses
}

#[test]
fn read_clause_shapes() {
    let c = clauses(
        "MATCH (a:Person {name: $n})-[r:KNOWS|LIKES]->(b) WHERE a.age > 3 \
         OPTIONAL MATCH (b)<-[:X]-(c) WITH a, count(*) AS k ORDER BY k DESC SKIP 1 LIMIT 2 \
         UNWIND [1,2] AS x RETURN DISTINCT a.name AS n, x",
    );
    assert_eq!(c.len(), 5);
    let Clause::Match {
        optional,
        pattern,
        where_,
        ..
    } = &c[0]
    else {
        panic!()
    };
    assert!(!optional && where_.is_some());
    assert_eq!(pattern[0].nodes.len(), 2);
    assert_eq!(pattern[0].rels[0].types.len(), 2);
    assert_eq!(pattern[0].rels[0].dir, Dir::Right);
    assert!(matches!(&c[1], Clause::Match { optional: true, .. }));
    let Clause::With { proj, .. } = &c[2] else {
        panic!()
    };
    assert_eq!(proj.items.len(), 2);
    assert!(proj.order[0].desc && proj.skip.is_some() && proj.limit.is_some());
    assert!(matches!(&c[3], Clause::Unwind { .. }));
    let Clause::Return(r) = &c[4] else { panic!() };
    assert!(r.distinct);
    assert_eq!(r.items[0].text, "a.name");
}

#[test]
fn spans_point_into_the_original_text() {
    let q = "MATCH (n) RETURN n.name AS x";
    let c = clauses(q);
    let Clause::Return(r) = &c[1] else { panic!() };
    assert_eq!(r.items[0].expr.span.text(q), Some("n.name"));
}

#[test]
fn expression_forms() {
    let c = clauses(
        "RETURN CASE WHEN 1 < 2 THEN 'a' ELSE 'b' END, [x IN [1,2] WHERE x > 1 | x * 2], \
         {a: 1}, n {.name, k: 1}, [1,2,3][1..2], -5, 1 + 2 * 3, x IS NOT NULL, n:Person, \
         toString(1), count(DISTINCT x), $p, 'a' STARTS WITH 'b', 2 IN [1,2], 0x1F",
    );
    let Clause::Return(r) = &c[0] else { panic!() };
    assert_eq!(r.items.len(), 15);
    assert!(matches!(r.items[0].expr.kind, ExprKind::Case { .. }));
    assert!(matches!(r.items[1].expr.kind, ExprKind::ListComp { .. }));
    assert!(matches!(r.items[3].expr.kind, ExprKind::MapProj(..)));
    assert!(matches!(r.items[4].expr.kind, ExprKind::Slice(..)));
    assert_eq!(r.items[5].expr.kind, ExprKind::Lit(Lit::Int(-5)));
    assert!(matches!(r.items[8].expr.kind, ExprKind::HasLabels(..)));
    assert!(matches!(
        &r.items[10].expr.kind,
        ExprKind::Call { distinct: true, .. }
    ));
    assert_eq!(r.items[14].expr.kind, ExprKind::Lit(Lit::Int(31)));
}

#[test]
fn write_clause_shapes() {
    let c = clauses(
        "MERGE (n:P {k: 1}) ON CREATE SET n.a = 1 ON MATCH SET n.b = 2 \
         SET n.x = 1, n += {y: 2}, n = {z: 3}, n:L1:L2 REMOVE n.x, n:L1 \
         CREATE (a)-[:T {w: 1}]->(b) DETACH DELETE n DELETE a",
    );
    let Clause::Merge {
        on_create,
        on_match,
        ..
    } = &c[0]
    else {
        panic!()
    };
    assert_eq!((on_create.len(), on_match.len()), (1, 1));
    let Clause::Set(items) = &c[1] else { panic!() };
    assert_eq!(items.len(), 4);
    assert!(matches!(items[1], SetItem::Merge { .. }));
    assert!(matches!(items[2], SetItem::Replace { .. }));
    assert!(matches!(&c[2], Clause::Remove(i) if i.len() == 2));
    assert!(matches!(&c[3], Clause::Create { .. }));
    assert!(matches!(&c[4], Clause::Delete { detach: true, .. }));
    assert!(matches!(&c[5], Clause::Delete { detach: false, .. }));
}

#[test]
fn call_subqueries_and_exists() {
    let c = clauses(
        "MATCH (c) CALL { WITH c OPTIONAL MATCH (p)-[:W]->(c) RETURN count(p) AS s } RETURN s",
    );
    let Clause::Subquery { body, imports, .. } = &c[1] else {
        panic!("{:?}", c[1])
    };
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].text, "c");
    assert_eq!(body.parts[0].clauses.len(), 2, "importing WITH removed");
    let c = clauses("CALL { MATCH (x) RETURN x } RETURN x");
    assert!(matches!(&c[0], Clause::Subquery { imports, .. } if imports.is_empty()));
    let c = clauses("MATCH (p) WHERE NOT EXISTS { (p)-[:W]->() } RETURN p");
    let Clause::Match {
        where_: Some(w), ..
    } = &c[0]
    else {
        panic!()
    };
    assert!(
        matches!(&w.kind, ExprKind::Unary(UnOp::Not, e) if matches!(e.kind, ExprKind::Exists(_)))
    );
}

// cypher-read "FOREACH rejected" / "LOAD CSV rejected" / "Negated label rejected"
#[test]
fn unsupported_constructs() {
    assert_eq!(
        unsupported("MATCH (n) FOREACH (x IN [1] | SET n.a = x)"),
        "FOREACH"
    );
    assert_eq!(
        unsupported("LOAD CSV FROM 'file:///x.csv' AS row RETURN row"),
        "LOAD CSV"
    );
    assert!(unsupported("MATCH (n:!Person) RETURN n").contains("label expression"));
    assert!(unsupported("MATCH (n:A&B) RETURN n").contains("label expression"));
    assert!(unsupported("MATCH (n:%) RETURN n").contains("label expression"));
    assert!(unsupported("CALL { MATCH (n) RETURN n } IN TRANSACTIONS").contains("IN TRANSACTIONS"));
    assert!(unsupported("USE mydb MATCH (n) RETURN n").contains("USE"));
    assert!(unsupported("CREATE INDEX FOR (n:P) ON (n.x)").contains("schema"));
    assert!(unsupported("DROP INDEX foo").contains("schema"));
    assert!(unsupported("SHOW INDEXES").contains("schema"));
    assert!(unsupported("MATCH (a) RETURN [(a)-->(b) | b.name]").contains("pattern comprehension"));
    assert!(unsupported("MATCH TRAIL (a)-[:X]->(b) RETURN a").contains("path modes"));
    assert!(unsupported("MATCH ((a)-[:X]->(b)){1,3} RETURN a").contains("quantified"));
}

// cypher-read "Variable-length pattern" / "shortestPath"
#[test]
fn path_constructs_are_unsupported_not_parse_errors() {
    assert_eq!(
        unsupported("MATCH (a {name:'Alice'})-[:knows*1..3]->(b) RETURN b"),
        "variable-length relationships"
    );
    assert_eq!(
        unsupported("MATCH (a)-[*]-(b) RETURN b"),
        "variable-length relationships"
    );
    assert_eq!(
        unsupported("MATCH p = shortestPath((a)-[:knows*]-(b)) RETURN p"),
        "shortestPath"
    );
    assert_eq!(
        unsupported("MATCH p = allShortestPaths((a)-[:knows*]-(b)) RETURN p"),
        "allShortestPaths"
    );
}

// cypher-read "Unclosed parenthesis"
#[test]
fn unclosed_parenthesis_span() {
    assert_eq!(parse_span("MATCH (n:Person RETURN n").start, 6);
}

// cypher-read "Span refers to original text after extensions"
#[test]
fn span_refers_to_original_text_after_extensions() {
    assert_eq!(parse_span("USE AS OF 3 MATCH (n RETURN n").start, 18);
    assert_eq!(
        parse_span("MATCH (n) USE AS OF 3 RETURN n").start,
        10,
        "a misplaced USE is left for the upstream parser"
    );
}

fn time(q: &str) -> TimeSel {
    parse(q).unwrap().parts[0].time.clone().unwrap()
}

// cypher-temporal-clauses: selectors
#[test]
fn use_selectors() {
    let t = time("USE AS OF 5 MATCH (n) RETURN n");
    assert!(matches!(t.tx, Some(TxClause::AsOf(TimeArg::Int(5), _))) && t.valid.is_none());
    let t = time("USE HISTORY MATCH (n) RETURN n");
    assert!(matches!(t.tx, Some(TxClause::History)));
    let t = time("USE AS OF $t VALID AT datetime('2026-01-01T00:00:00Z') MATCH (n) RETURN n");
    assert!(matches!(t.tx, Some(TxClause::AsOf(TimeArg::Param(ref p), _)) if p == "t"));
    assert!(matches!(t.valid, Some((TimeArg::DateTime(ref s), _)) if s == "2026-01-01T00:00:00Z"));
    let t = time("USE VALID AT date('2021-06-01') MATCH (n) RETURN n");
    assert!(t.tx.is_none() && matches!(t.valid, Some((TimeArg::Date(_), _))));
    let t = time("USE HISTORY VALID AT 10 MATCH (n) RETURN n");
    assert!(matches!(t.tx, Some(TxClause::History)) && t.valid.is_some());
}

#[test]
fn use_selector_errors() {
    // two transaction selectors, float literal
    assert!(matches!(
        parse("USE AS OF 5 HISTORY MATCH (n) RETURN n"),
        Err(CypherError::Parse { .. })
    ));
    assert!(matches!(
        parse("USE AS OF 5.5 MATCH (n) RETURN n"),
        Err(CypherError::Parse { .. })
    ));
    assert!(matches!(
        parse("USE AS OF 'x' MATCH (n) RETURN n"),
        Err(CypherError::Parse { .. })
    ));
}

#[test]
fn scope_starts() {
    let q = parse("USE AS OF 1 MATCH (n) RETURN n UNION USE AS OF 2 MATCH (n) RETURN n").unwrap();
    assert!(q.parts[0].time.is_some() && q.parts[1].time.is_some());
    let q = parse("MATCH (c) CALL { USE AS OF 5 WITH c MATCH (x) RETURN x } RETURN x").unwrap();
    let Clause::Subquery { body, .. } = &q.parts[0].clauses[1] else {
        panic!()
    };
    assert!(body.parts[0].time.is_some());
    let q = parse("MATCH (c) CALL { WITH c USE AS OF 5 MATCH (x) RETURN x } RETURN x").unwrap();
    let Clause::Subquery { body, imports, .. } = &q.parts[0].clauses[1] else {
        panic!()
    };
    assert!(body.parts[0].time.is_some() && imports.len() == 1);
}

// cypher-read REPEATABLE ELEMENTS / DIFFERENT RELATIONSHIPS
#[test]
fn match_modes() {
    let c =
        clauses("MATCH REPEATABLE ELEMENTS (x)-[r1:knows]-(y)-[r2:knows]-(z) RETURN count(*) AS c");
    assert!(matches!(
        &c[0],
        Clause::Match {
            mode: MatchModeExt::Repeatable,
            ..
        }
    ));
    let c = clauses("MATCH DIFFERENT RELATIONSHIPS (x)-[r1:knows]-(y) RETURN count(*) AS c");
    assert!(matches!(
        &c[0],
        Clause::Match {
            mode: MatchModeExt::Default,
            ..
        }
    ));
    let c = clauses("MATCH (n) OPTIONAL MATCH REPEATABLE ELEMENTS (n)-[r]-(m) RETURN n");
    assert!(matches!(
        &c[1],
        Clause::Match {
            optional: true,
            mode: MatchModeExt::Repeatable,
            ..
        }
    ));
}

#[test]
fn extension_inside_a_string_or_comment_is_left_alone() {
    let q = "MATCH (n {t: 'USE AS OF 3'}) // USE AS OF 4\nRETURN n";
    let p = prepass::run(q).unwrap();
    assert!(p.scopes.is_empty());
    assert_eq!(p.blanked, q);
}

fn inject(q: &str, k: u8) -> String {
    match k % 4 {
        0 => format!("USE AS OF {} {q}", k as u32 + 1),
        1 => format!("USE HISTORY VALID AT {} {q}", k),
        2 => q.replacen("MATCH", "MATCH REPEATABLE ELEMENTS", 1),
        _ => q.replacen("MATCH", "MATCH DIFFERENT RELATIONSHIPS", 1),
    }
}

proptest! {
    // Span refers to original text after extensions: blanking never moves bytes.
    #[test]
    fn blanking_preserves_offsets(k in 0u8..200, name in "[a-z]{1,6}", pad in 0usize..4) {
        let base = format!("MATCH ({name}:P)-[r:T]->(m) WHERE {name}.x = 'it''s' RETURN {name}{}", " ".repeat(pad));
        let q = inject(&base, k);
        let p = prepass::run(&q).unwrap();
        prop_assert_eq!(p.blanked.len(), q.len());
        // identical outside the blanked regions
        let tail_q = &q[q.len() - 30..];
        let tail_b = &p.blanked[q.len() - 30..];
        prop_assert_eq!(tail_q, tail_b);
        // every upstream span slices the same text
        let ast = parse(&q).unwrap();
        let Clause::Return(r) = ast.parts[0].clauses.last().unwrap() else { panic!() };
        prop_assert_eq!(r.items[0].expr.span.text(&q), Some(name.as_str()));
        let up = open_cypher::parse(&p.blanked).unwrap();
        prop_assert!(up.program.span.end <= q.len());
    }
}
