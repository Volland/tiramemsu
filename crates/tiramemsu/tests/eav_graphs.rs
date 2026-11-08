//! The worked example of the article `article/eav-graphs/` (Datomic-style EAV
//! with Datalog on Tiramemsu, then what statement ids and valid time add):
//! every program, query and result the article shows is checked here, one
//! test per section of the article.
//!
//! Story (transaction instants UTC, 2026): HR imports a small engineering org
//! on 6 January; Ada is promoted on 10 February; on 4 May HR records that Ada
//! split her time between storage and search from 1 March; a chat agent adds
//! a guess about Cleo on 6 May; Dan is named the next lead of platform on
//! 1 June.
#![cfg(all(feature = "datalog", feature = "sparql", feature = "cypher"))]

use std::sync::Arc;

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

/// Epoch ms of `YYYY-MM-DD` (midnight UTC) or a full RFC 3339 instant.
fn ms(d: &str) -> i64 {
    let full = if d.len() == 10 {
        format!("{d}T00:00:00Z")
    } else {
        d.to_string()
    };
    value::parse_datetime(&full).unwrap().0
}

/// Everyone in a team or any team below it, over valid time.
const ORG: &str = "
within(T, U) :- v:partOf(T, U).
within(T, W) :- within(T, U), v:partOf(U, W).
inOrg(P, T)  :- v:memberOf(P, T).
inOrg(P, U)  :- v:memberOf(P, T), within(T, U).
";

/// The same rules with a stored head, for materialization.
const ORG_STORED: &str = "
within(T, U) :- v:partOf(T, U).
within(T, W) :- within(T, U), v:partOf(U, W).
v:inOrg(P, T) :- v:memberOf(P, T).
v:inOrg(P, U) :- v:memberOf(P, T), within(T, U).
";

const ONE_LEAD: &str = "constraint one_lead error :- v:leads(X, T), v:leads(Y, T), X != Y.\n";

struct Company {
    _dir: tempfile::TempDir,
    db: Db,
    clock: Arc<ManualClock>,
    /// Transaction numbers of the story's commits.
    t_import: u64,
    t_promote: u64,
    t_split: u64,
    /// Cleo's membership of search from the import.
    cleo_search: Eid,
}

impl Company {
    fn at(&self, d: &str) {
        self.clock.set(ms(d));
    }

    fn tx<R>(&self, f: impl FnOnce(&mut Tx<'_>) -> Result<R>) -> (R, u64) {
        commit(&self.db, f)
    }

    fn goal(&self, view: &View<'_>, program: &str, goal: &str) -> Vec<String> {
        render(
            &view
                .datalog(
                    &DatalogProgram::Text(program.to_string()),
                    Some(goal),
                    &DatalogOptions::default(),
                )
                .unwrap_or_else(|e| panic!("{goal}: {e}")),
        )
    }

    fn sparql(&self, view: &View<'_>, q: &str) -> Vec<Vec<String>> {
        match view.sparql(q).unwrap_or_else(|e| panic!("{q}: {e}")) {
            SparqlResult::Solutions(s) => s
                .rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| c.as_ref().map(|v| v.to_string()).unwrap_or_default())
                        .collect()
                })
                .collect(),
            other => panic!("not solutions: {other:?}"),
        }
    }

    /// The 2026 story up to 4 May.
    fn new() -> Company {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(ms("2026-01-05T09:00:00Z")));
        let db = Db::open(
            dir.path().join("org.db"),
            OpenOptions {
                clock: clock.clone(),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        // Schema: attributes are predicates, and their flags are statements too.
        commit(&db, |t| {
            t.assert(
                v("email"),
                Value::iri(vocab::SYS_UNIQUE),
                Value::Bool(true),
                Valid::ALWAYS,
            )?;
            t.assert(
                v("title"),
                Value::iri(vocab::SYS_CARDINALITY),
                Value::iri(vocab::SYS_ONE),
                Valid::ALWAYS,
            )?;
            Ok(())
        });
        clock.set(ms("2026-01-06T10:00:00Z"));
        let ((ada_storage, cleo_search), t_import) = commit(&db, |t| {
            hr(t, "hr-import")?;
            for (team, parent) in [
                ("storage", "platform"),
                ("search", "platform"),
                ("platform", "eng"),
            ] {
                t.assert(v(team), v("partOf"), v(parent), Valid::ALWAYS)?;
            }
            for (team, name) in [
                ("storage", "Storage"),
                ("search", "Search"),
                ("platform", "Platform"),
                ("eng", "Engineering"),
            ] {
                t.assert(v(team), v("name"), Value::str(name), Valid::ALWAYS)?;
            }
            for (who, name, email, title) in [
                ("ada", "Ada", "ada@example.org", "Engineer"),
                ("bob", "Bob", "bob@example.org", "Lead"),
                ("cleo", "Cleo", "cleo@example.org", "Engineer"),
            ] {
                t.assert(v(who), v("name"), Value::str(name), Valid::ALWAYS)?;
                t.assert(v(who), v("email"), Value::str(email), Valid::ALWAYS)?;
                t.assert(v(who), v("title"), Value::str(title), Valid::ALWAYS)?;
            }
            let a = t
                .assert(
                    v("ada"),
                    v("memberOf"),
                    v("storage"),
                    Valid::from(ms("2024-03-01")),
                )?
                .eid();
            t.add_to_graph(a, v("hr"), AssertOpts::default())?;
            let b = t
                .assert(
                    v("bob"),
                    v("memberOf"),
                    v("platform"),
                    Valid::from(ms("2023-01-01")),
                )?
                .eid();
            t.add_to_graph(b, v("hr"), AssertOpts::default())?;
            let cl = t
                .assert(
                    v("cleo"),
                    v("memberOf"),
                    v("search"),
                    Valid::from(ms("2025-06-01")),
                )?
                .eid();
            t.add_to_graph(cl, v("hr"), AssertOpts::default())?;
            t.assert(
                v("bob"),
                v("leads"),
                v("platform"),
                Valid::from(ms("2023-01-01")),
            )?;
            Ok((a, cl))
        });
        // Ada is promoted: cardinality one replaces the old title.
        clock.set(ms("2026-02-10T15:00:00Z"));
        let t_promote = commit(&db, |t| {
            hr(t, "promotion form")?;
            t.assert(
                v("ada"),
                v("title"),
                Value::str("Senior Engineer"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .1;
        // On 4 May HR records that from 1 March Ada splits her time 40/60.
        clock.set(ms("2026-05-04T11:00:00Z"));
        let t_split = commit(&db, |t| {
            hr(t, "reorg sheet")?;
            t.supersede(
                ada_storage,
                Patch {
                    o: None,
                    v_from: None,
                    v_to: Some(Some(ms("2026-03-01"))),
                },
            )?;
            for (team, share) in [("storage", 40), ("search", 60)] {
                let e = t
                    .assert(
                        v("ada"),
                        v("memberOf"),
                        v(team),
                        Valid::from(ms("2026-03-01")),
                    )?
                    .eid();
                t.assert(
                    Value::Stmt(e),
                    v("allocation"),
                    Value::Int(share),
                    Valid::ALWAYS,
                )?;
                t.add_to_graph(e, v("hr"), AssertOpts::default())?;
            }
            Ok(())
        })
        .1;
        Company {
            _dir: dir,
            db,
            clock,
            t_import,
            t_promote,
            t_split,
            cleo_search,
        }
    }

    /// A chat agent's guess on 6 May, in its own graph, with a confidence.
    fn agent_guess(&self) -> Eid {
        self.at("2026-05-06T08:30:00Z");
        self.tx(|t| {
            t.meta(Value::iri(vocab::SYS_AUTHOR), v("chat-agent"))?;
            t.meta(
                Value::iri(vocab::SYS_SOURCE),
                Value::str("#storage-team thread"),
            )?;
            let e = t
                .assert(
                    v("cleo"),
                    v("memberOf"),
                    v("storage"),
                    Valid::from(ms("2026-05-01")),
                )?
                .eid();
            t.assert(
                Value::Stmt(e),
                v("confidence"),
                Value::Double(0.6),
                Valid::ALWAYS,
            )?;
            t.add_to_graph(e, v("agent-notes"), AssertOpts::default())?;
            Ok(e)
        })
        .0
    }
}

fn commit<R>(db: &Db, f: impl FnOnce(&mut Tx<'_>) -> Result<R>) -> (R, u64) {
    let mut out = None;
    let rep = db
        .transact(TxOptions::default(), |t| {
            out = Some(f(t)?);
            Ok(())
        })
        .unwrap();
    (out.unwrap(), rep.t.0)
}

fn hr(t: &mut Tx<'_>, source: &str) -> Result<()> {
    t.meta(Value::iri(vocab::SYS_AUTHOR), v("hr"))?;
    t.meta(Value::iri(vocab::SYS_SOURCE), Value::str(source))?;
    Ok(())
}

/// A row as `value, value, … @ [from, to) …` with dates as `YYYY-MM-DD`.
fn render(r: &DatalogResult) -> Vec<String> {
    r.rows
        .iter()
        .map(|row| {
            let vals: Vec<String> = row.values.iter().map(short).collect();
            let ivs: Vec<String> = row.valid.iter().map(interval).collect();
            if ivs == ["always"] {
                vals.join(", ")
            } else {
                format!("{} @ {}", vals.join(", "), ivs.join(" "))
            }
        })
        .collect()
}

fn short(t: &DatalogTerm) -> String {
    match t {
        DatalogTerm::Value(Value::Int(i)) => i.to_string(),
        DatalogTerm::Value(Value::Double(d)) => d.to_string(),
        DatalogTerm::Value(Value::DateTime { ms, .. }) => Value::DateTime {
            ms: *ms,
            tz: Some(0),
        }
        .to_string()[1..25]
            .to_string(),
        other => other
            .to_string()
            .replace("<urn:tiramemsu:v:", "v:")
            .replace("<urn:tiramemsu:", "")
            .replace('>', ""),
    }
}

fn day(ms: Option<i64>) -> String {
    match ms {
        None => "inf".into(),
        Some(m) => Value::DateTime { ms: m, tz: Some(0) }.to_string()[1..11].to_string(),
    }
}

fn interval(v: &Valid) -> String {
    if v.from.is_none() && v.to.is_none() {
        return "always".into();
    }
    format!("[{}, {})", day(v.from), day(v.to))
}

// ---------------------------------------------------------- the EAV setup

/// "A datom, a statement": the Datomic-style find with a join on two
/// attributes of one entity.
#[test]
fn find_by_attribute() {
    let c = Company::new();
    let rows = c.goal(&c.db.now(), "", "v:title(E, \"Lead\"), v:name(E, N)");
    assert_eq!(rows, ["v:bob, \"Bob\""]);
}

/// "Cardinality one and the past": the promotion replaced the title, the old
/// one is still there as of the import, and history shows why it ended.
#[test]
fn cardinality_one_and_as_of() {
    let c = Company::new();
    let q = "v:title(v:ada, T)";
    assert_eq!(c.goal(&c.db.now(), "", q), ["\"Senior Engineer\""]);
    assert_eq!(
        c.goal(&c.db.as_of(TimeRef::Tx(c.t_import)), "", q),
        ["\"Engineer\""]
    );
    let h = c.db.history();
    let title = h.encode(&v("title")).unwrap().unwrap();
    let ada = h.encode(&v("ada")).unwrap().unwrap();
    let mut versions: Vec<(String, u64, Option<u64>, Option<RetKind>)> = h
        .triples(Some(ada), Some(title), None)
        .unwrap()
        .iter()
        .map(|t| {
            (
                h.decode(t.o).unwrap().to_string(),
                t.t_add.0,
                t.t_ret.map(|x| x.0),
                t.ret_kind,
            )
        })
        .collect();
    versions.sort_by_key(|v| v.1);
    assert_eq!(
        versions,
        [
            (
                "\"Engineer\"".into(),
                c.t_import,
                Some(c.t_promote),
                Some(RetKind::Cardinality)
            ),
            ("\"Senior Engineer\"".into(), c.t_promote, None, None),
        ]
    );
    // the instant view: what we believed on 1 February
    let feb = c.db.as_of(TimeRef::Instant(ms("2026-02-01T00:00:00Z")));
    assert_eq!(c.goal(&feb, "", q), ["\"Engineer\""]);
}

/// "Who wrote it": the transaction is an entity, and its metadata joins
/// like any other attribute.
#[test]
fn transaction_metadata() {
    let c = Company::new();
    let rows = c.goal(
        &c.db.now(),
        "",
        "v:title(v:ada, X) @ E, added(E, T), sys:author(T, A), sys:source(T, S), tx_instant(T, Ms)",
    );
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert!(r.starts_with("\"Senior Engineer\", "), "{r}");
    assert!(r.contains("v:hr, \"promotion form\""), "{r}");
    assert_eq!(
        r,
        "\"Senior Engineer\", stmt:30, tx:3, v:hr, \"promotion form\", 2026-02-10T15:00:00.000Z"
    );
}

/// "Rules": a recursive rule over ref attributes walks the team tree.
#[test]
fn recursive_rules_over_refs() {
    let c = Company::new();
    let view = c.as_of_before_split();
    assert_eq!(
        c.goal(&view, ORG, "inOrg(P, v:eng)"),
        [
            "v:ada @ [2024-03-01, inf)",
            "v:bob @ [2023-01-01, inf)",
            "v:cleo @ [2025-06-01, inf)",
        ]
    );
}

impl Company {
    fn as_of_before_split(&self) -> View<'_> {
        self.db.as_of(TimeRef::Tx(self.t_split - 1))
    }
}

/// "Speculation": `with` runs a write and a query without keeping the write.
#[test]
fn what_if_with() {
    let c = Company::new();
    let seen =
        c.db.with(
            |t| {
                t.assert(
                    v("dan"),
                    v("memberOf"),
                    v("search"),
                    Valid::from(ms("2026-06-01")),
                )?;
                Ok(())
            },
            |view| Ok(c.goal(view, ORG, "inOrg(P, v:platform)").len()),
        )
        .unwrap();
    assert_eq!(seen, 4);
    assert_eq!(c.goal(&c.db.now(), ORG, "inOrg(P, v:platform)").len(), 3);
}

// ------------------------------------------------------- the graph power-ups

const INT60: &str = "\"60\"^^<http://www.w3.org/2001/XMLSchema#integer>";
const INT40: &str = "\"40\"^^<http://www.w3.org/2001/XMLSchema#integer>";

/// "Edges with properties": the allocation is a statement about the
/// membership statement, read in Datalog, SPARQL and Cypher.
#[test]
fn edge_properties() {
    let c = Company::new();
    let now = c.db.now();
    assert_eq!(
        c.goal(&now, "", "v:memberOf(v:ada, T) @ E, v:allocation(E, A)"),
        [
            "v:search, stmt:38, 60 @ [2026-03-01, inf)",
            "v:storage, stmt:35, 40 @ [2026-03-01, inf)"
        ]
    );
    assert_eq!(
        c.sparql(
            &now,
            "SELECT ?team ?a WHERE { v:ada v:memberOf ?team {| v:allocation ?a |} } ORDER BY ?team"
        ),
        [
            vec!["<urn:tiramemsu:v:search>".to_string(), INT60.into()],
            vec!["<urn:tiramemsu:v:storage>".to_string(), INT40.into()],
        ]
    );
    let r = now
        .cypher(
            "MATCH (p {name: 'Ada'})-[r:memberOf]->(t) WHERE r.allocation IS NOT NULL \
             RETURN t.name AS team, r.allocation AS share ORDER BY share",
            &CypherParams::default(),
        )
        .unwrap();
    assert_eq!(
        format!("{:?}", r.rows),
        r#"[[String("Storage"), Integer(40)], [String("Search"), Integer(60)]]"#
    );
}

/// "Valid time": one membership in storage coalesces across the split under
/// set semantics, two statements with `@ E`; Ada and Cleo share search only
/// from 1 March; and before 4 May the store did not know it yet.
#[test]
fn valid_time_and_two_clocks() {
    let c = Company::new();
    let now = c.db.now();
    assert_eq!(
        c.goal(&now, "", "v:memberOf(v:ada, T)"),
        [
            "v:search @ [2026-03-01, inf)",
            "v:storage @ [2024-03-01, inf)"
        ]
    );
    assert_eq!(
        c.goal(&now, "", "v:memberOf(v:ada, v:storage) @ E").len(),
        2
    );
    let pairs = "together(X, Y, T) :- v:memberOf(X, T), v:memberOf(Y, T), X != Y.";
    assert_eq!(
        c.goal(&now, pairs, "together(v:ada, Y, T)"),
        ["v:cleo, v:search @ [2026-03-01, inf)"]
    );
    // the same question as of 1 April: the move was not recorded yet
    let april = c.db.as_of(TimeRef::Instant(ms("2026-04-01T00:00:00Z")));
    assert!(c.goal(&april, pairs, "together(v:ada, Y, T)").is_empty());
}

/// "Named graphs and confidence": the agent's guess is a statement in its own
/// graph; a rule can trust only HR's graph, or weigh the guess.
#[test]
fn graphs_and_confidence() {
    let c = Company::new();
    c.agent_guess();
    let now = c.db.now();
    let trusted = "hrMember(P, T) :- v:memberOf(P, T) @ E, in_graph(E, v:hr).";
    assert_eq!(
        c.goal(&now, trusted, "hrMember(v:cleo, T)"),
        ["v:search @ [2025-06-01, inf)"]
    );
    assert_eq!(
        c.goal(&now, "", "v:memberOf(v:cleo, T)"),
        [
            "v:search @ [2025-06-01, inf)",
            "v:storage @ [2026-05-01, inf)"
        ]
    );
    let guess = c.goal(
        &now,
        "",
        "v:memberOf(P, T) @ E, v:confidence(E, K), added(E, X), sys:author(X, A)",
    );
    assert_eq!(guess.len(), 1);
    assert!(guess[0].starts_with("v:cleo, v:storage, "), "{}", guess[0]);
    assert_eq!(
        guess[0],
        "v:cleo, v:storage, stmt:43, 0.6, tx:5, v:chat-agent @ [2026-05-01, inf)"
    );
    assert!(
        guess[0].ends_with("v:chat-agent @ [2026-05-01, inf)"),
        "{}",
        guess[0]
    );
}

/// "Constraints at commit": two leads of one team at once are refused, a
/// hand-over at a date is accepted.
#[test]
fn one_lead_at_a_time() {
    let c = Company::new();
    c.at("2026-06-01T09:00:00Z");
    c.tx(|t| {
        t.store_program(v("policies"), ONE_LEAD)?;
        t.enforce(v("policies"), true)?;
        Ok(())
    });
    let clash = c.db.transact(TxOptions::default(), |t| {
        t.assert(
            v("dan"),
            v("leads"),
            v("platform"),
            Valid::from(ms("2026-07-01")),
        )?;
        Ok(())
    });
    match clash {
        Err(Error::ConstraintViolated { constraint, .. }) => assert_eq!(constraint, "one_lead"),
        other => panic!("{other:?}"),
    }
    let now = c.db.now();
    let bob_leads = now
        .triples(
            Some(now.encode(&v("bob")).unwrap().unwrap()),
            Some(now.encode(&v("leads")).unwrap().unwrap()),
            None,
        )
        .unwrap()[0]
        .eid;
    c.tx(|t| {
        t.supersede(
            bob_leads,
            Patch {
                o: None,
                v_from: None,
                v_to: Some(Some(ms("2026-07-01"))),
            },
        )?;
        t.assert(
            v("dan"),
            v("leads"),
            v("platform"),
            Valid::from(ms("2026-07-01")),
        )?;
        Ok(())
    });
    assert_eq!(
        c.goal(&c.db.now(), "", "v:leads(X, v:platform)"),
        [
            "v:bob @ [2023-01-01, 2026-07-01)",
            "v:dan @ [2026-07-01, inf)"
        ]
    );
}

/// "Derived facts remember why": materialized conclusions cite their
/// premises, and fall when a premise is retracted.
#[test]
fn materialized_with_support() {
    let c = Company::new();
    c.tx(|t| t.store_program(v("org"), ORG_STORED));
    let rep =
        c.db.materialize_program(&v("org"), &MaterializeProgramOptions::default())
            .unwrap();
    let rows = |c: &Company| c.goal(&c.db.now(), "", "v:inOrg(v:cleo, T)");
    assert_eq!(
        rows(&c),
        [
            "v:eng @ [2025-06-01, inf)",
            "v:platform @ [2025-06-01, inf)",
            "v:search @ [2025-06-01, inf)"
        ]
    );
    assert!(!rep.derived.is_empty());
    let r =
        c.db.transact(TxOptions::default(), |t| {
            t.retract(c.cleo_search)?;
            Ok(())
        })
        .unwrap();
    assert!(r.retracted.iter().any(|(_, k)| *k == RetKind::Cascade));
    assert_eq!(rows(&c), Vec::<String>::new());
}

/// "One store, every surface": a SPARQL property path over the same refs,
/// and SPARQL and Cypher queries joining a stored Datalog goal.
#[test]
fn same_facts_from_sparql() {
    let c = Company::new();
    let view = c.as_of_before_split();
    assert_eq!(
        c.sparql(
            &view,
            "SELECT ?name WHERE { ?p v:memberOf/v:partOf* v:eng ; v:name ?name } ORDER BY ?name"
        ),
        [
            vec!["\"Ada\"".to_string()],
            vec!["\"Bob\"".into()],
            vec!["\"Cleo\"".into()]
        ]
    );
    c.tx(|t| t.store_program(v("org"), ORG));
    assert_eq!(
        c.sparql(
            &c.db.now(),
            "SELECT ?mail WHERE { \
               SERVICE <urn:tiramemsu:dl:query> { [] dl:program v:org ; dl:goal \"inOrg(P, v:search)\" } \
               ?P v:email ?mail } ORDER BY ?mail"
        ),
        [vec!["\"ada@example.org\"".to_string()], vec!["\"cleo@example.org\"".into()]]
    );
    let r =
        c.db.now()
            .cypher(
                "CALL tiramemsu.datalog.query('v:org', 'inOrg(P, v:search)') \
             YIELD P, validFrom, validTo RETURN P.email AS mail, validFrom ORDER BY mail",
                &CypherParams::default(),
            )
            .unwrap();
    assert_eq!(
        format!("{:?}", r.rows),
        format!(
            "[[String(\"ada@example.org\"), DateTime {{ ms: {}, tz: 0 }}], \
             [String(\"cleo@example.org\"), DateTime {{ ms: {}, tz: 0 }}]]",
            ms("2026-03-01"),
            ms("2025-06-01")
        )
    );
}
