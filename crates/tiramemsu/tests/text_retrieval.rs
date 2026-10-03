//! Text recall with evidence ranking (OpenSpec change `add-text-retrieval`):
//! view-aware recall over dictionary and inline strings, deterministic ranking with
//! absent evidence reported as absent, the derived index lifecycle (format
//! migration, rebuild, hosts without FTS5) and the SPARQL and Cypher entrypoints.

use std::path::{Path, PathBuf};

use tiramemsu::*;

fn v(s: &str) -> Value {
    Value::iri(format!("urn:tiramemsu:v:{s}"))
}

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("text.db");
    (d, p)
}

fn indexed() -> OpenOptions {
    OpenOptions {
        text_index: true,
        ..OpenOptions::default()
    }
}

fn open(p: &Path) -> Db {
    Db::open(p, indexed()).unwrap()
}

fn eids(hits: &[TextHit]) -> Vec<Eid> {
    hits.iter().map(|h| h.eid).collect()
}

/// Every row of the graph tables, for "history is untouched" checks.
fn history_rows(db: &Db) -> Vec<Vec<SqlValue>> {
    let mut out = Vec::new();
    for t in ["triple", "term", "tx"] {
        out.extend(
            db.read_sql(&format!("SELECT * FROM {t} ORDER BY 1"))
                .unwrap(),
        );
    }
    out
}

fn count(db: &Db, sql: &str) -> i64 {
    db.read_sql(sql).unwrap()[0][0].as_i64().unwrap()
}

// @lat: [[tests#Text Retrieval#Inline Text Is Recalled Without A Dictionary Row]]
#[test]
fn inline_short_strings_are_recalled_without_a_dictionary_row() {
    let (_d, p) = tmp();
    let db = open(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(v("bob"), v("drinks"), Value::str("tea"), Valid::ALWAYS)?; // inline
            tx.assert(
                v("ann"),
                v("drinks"),
                Value::str("green tea, no sugar"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let terms_before = count(&db, "SELECT count(*) FROM term");
    let hits = db.now().text_search(&TextQuery::new("tea")).unwrap();
    let mut got = eids(&hits);
    got.sort();
    assert_eq!(got, r.asserted);
    let bob = hits.iter().find(|h| h.eid == r.asserted[0]).unwrap();
    assert_eq!(bob.o.tag().unwrap(), Tag::ShortStr);
    assert_eq!(bob.text, "tea");
    // the inline value got no dictionary row, before or after recall and rebuild
    let no_row = "SELECT count(*) FROM term WHERE lex = 'tea'";
    assert_eq!(count(&db, no_row), 0);
    db.rebuild_text_index().unwrap();
    assert_eq!(count(&db, no_row), 0);
    assert_eq!(count(&db, "SELECT count(*) FROM term"), terms_before);
    // the index keys it by its full ObjectId
    let key = format!(
        "SELECT count(*) FROM term_fts WHERE rowid = {}",
        bob.o.raw()
    );
    assert_eq!(count(&db, &key), 1);
}

// @lat: [[tests#Text Retrieval#Language Tagged Text Keeps Its Tag]]
#[test]
fn language_tagged_strings_keep_their_tag_and_typed_literals_are_not_searchable() {
    let (_d, p) = tmp();
    let db = open(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            let lang = Value::LangStr {
                lex: "Café au lait".into(),
                lang: "fr".into(),
            };
            tx.assert(v("ann"), v("likes"), lang, Valid::ALWAYS)?;
            let typed = Value::Typed {
                lex: "cafe".into(),
                datatype: "urn:example:code".into(),
            };
            tx.assert(v("ann"), v("code"), typed, Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    // diacritics are folded: "cafe" matches "Café"
    let hits = db.now().text_search(&TextQuery::new("cafe")).unwrap();
    assert_eq!(eids(&hits), vec![r.asserted[0]]);
    assert_eq!(hits[0].lang.as_deref(), Some("fr"));
    assert_eq!(hits[0].text, "Café au lait");
    // phrase and any-word modes, and a prefix
    let phrase = TextQuery {
        mode: TextMode::Phrase,
        ..TextQuery::new("au lait")
    };
    assert_eq!(db.now().text_search(&phrase).unwrap().len(), 1);
    let wrong_order = TextQuery {
        mode: TextMode::Phrase,
        ..TextQuery::new("lait au")
    };
    assert!(db.now().text_search(&wrong_order).unwrap().is_empty());
    let any = TextQuery {
        mode: TextMode::Any,
        ..TextQuery::new("nothing lait")
    };
    assert_eq!(db.now().text_search(&any).unwrap().len(), 1);
    assert!(db
        .now()
        .text_search(&TextQuery::new("nothing lait"))
        .unwrap()
        .is_empty());
    assert_eq!(
        db.now().text_search(&TextQuery::new("caf*")).unwrap().len(),
        1
    );
    // query text is never FTS5 syntax, and a query without a word is invalid
    assert!(db
        .now()
        .text_search(&TextQuery::new("lait OR NEAR("))
        .unwrap()
        .is_empty());
    assert!(matches!(
        db.now().text_search(&TextQuery::new(" * ")),
        Err(Error::InvalidQuery { .. })
    ));
}

// @lat: [[tests#Text Retrieval#Retracted Text Leaves Now Recall]]
#[test]
fn retracted_text_is_absent_now_and_present_as_of_before_the_retraction() {
    let (_d, p) = tmp();
    let db = open(&p);
    let r1 = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(
                v("ann"),
                v("note"),
                Value::str("allergic to peanuts"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let e = r1.asserted[0];
    let r2 = db
        .transact(TxOptions::default(), |tx| tx.retract(e).map(|_| ()))
        .unwrap();
    let q = TextQuery::new("peanuts");
    assert!(db.now().text_search(&q).unwrap().is_empty());
    let then = db.as_of(TimeRef::Tx(r1.t.0)).text_search(&q).unwrap();
    assert_eq!(eids(&then), vec![e]);
    assert!(db
        .as_of(TimeRef::Tx(r2.t.0))
        .text_search(&q)
        .unwrap()
        .is_empty());
    assert_eq!(eids(&db.history().text_search(&q).unwrap()), vec![e]);
    // an instant resolves like any as-of view
    let at = db
        .as_of(TimeRef::Instant(r1.instant))
        .text_search(&q)
        .unwrap();
    assert_eq!(eids(&at), vec![e]);
}

// @lat: [[tests#Text Retrieval#Recall Honors Graph And Valid Time]]
#[test]
fn every_hit_satisfies_the_graph_and_the_valid_instant() {
    let (_d, p) = tmp();
    let db = open(&p);
    // (graph, valid interval) for four "standup" notes, plus one in no graph
    let cases = [
        ("g1", Valid::between(0, 100)),
        ("g1", Valid::between(100, 200)),
        ("g2", Valid::between(0, 100)),
        ("g1", Valid::ALWAYS),
    ];
    let mut eid_of = Vec::new();
    db.transact(TxOptions::default(), |tx| {
        for (i, (g, valid)) in cases.iter().enumerate() {
            let e = tx
                .assert(
                    v(&format!("m{i}")),
                    v("note"),
                    Value::str("daily standup notes"),
                    *valid,
                )?
                .eid();
            tx.add_to_graph(e, v(g), AssertOpts::default())?;
            eid_of.push(e);
        }
        tx.assert(
            v("m9"),
            v("note"),
            Value::str("daily standup notes"),
            Valid::ALWAYS,
        )?;
        Ok(())
    })
    .unwrap();
    let g1 = db.now().encode(&v("g1")).unwrap().unwrap();
    let q = TextQuery {
        graphs: Some(vec![g1]),
        ..TextQuery::new("standup")
    };
    let view = db.now().valid_at(50);
    let hits = view.text_search(&q).unwrap();
    let mut got = eids(&hits);
    got.sort();
    assert_eq!(got, vec![eid_of[0], eid_of[3]]);
    let members = db.now().graph_members(g1).unwrap();
    for h in &hits {
        assert!(members.contains(&h.eid));
        let t = &db.now().triples(Some(h.s), Some(h.p), Some(h.o)).unwrap()[0];
        assert!(t.v_from.is_none_or(|f| f <= 50) && t.v_to.is_none_or(|to| to > 50));
    }
    // graphs without the valid filter, the valid filter without graphs
    assert_eq!(db.now().text_search(&q).unwrap().len(), 3);
    assert_eq!(
        view.text_search(&TextQuery::new("standup")).unwrap().len(),
        4
    );
    // an empty graph list and a graph nobody wrote match nothing
    let none = TextQuery {
        graphs: Some(vec![]),
        ..TextQuery::new("standup")
    };
    assert!(db.now().text_search(&none).unwrap().is_empty());
    // a predicate filter
    let note = db.now().encode(&v("note")).unwrap().unwrap();
    let other = TextQuery {
        predicates: Some(vec![g1]),
        ..TextQuery::new("standup")
    };
    assert!(db.now().text_search(&other).unwrap().is_empty());
    let by_note = TextQuery {
        predicates: Some(vec![note]),
        ..TextQuery::new("standup")
    };
    assert_eq!(db.now().text_search(&by_note).unwrap().len(), 5);
}

// @lat: [[tests#Text Retrieval#Absent Confidence Is Reported As Absent]]
#[test]
fn a_hit_without_a_confidence_layer_reports_it_as_absent() {
    let (_d, p) = tmp();
    let db = open(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            let a = tx.assert(
                v("ann"),
                v("works"),
                Value::str("works at acme"),
                Valid::ALWAYS,
            )?;
            tx.assert(a.eid(), v("confidence"), Value::Double(0.4), Valid::ALWAYS)?;
            tx.assert(a.eid(), v("confidence"), Value::Int(1), Valid::ALWAYS)?;
            tx.assert(
                v("bob"),
                v("works"),
                Value::str("works at acme"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let (ann, bob) = (r.asserted[0], r.asserted[3]);
    db.transact(TxOptions::default(), |tx| tx.confirm(bob).map(|_| ()))
        .unwrap();
    let hits = db.now().text_search(&TextQuery::new("acme")).unwrap();
    assert_eq!(eids(&hits), vec![ann, bob]); // confidence first, absent last
    assert_eq!(hits[0].evidence.confidence, Some(1.0)); // the largest value
    assert_eq!(hits[0].evidence.confirmations, 0);
    assert_eq!(hits[1].evidence.confidence, None);
    assert_eq!(hits[1].evidence.confirmations, 1);
    assert_eq!(hits[0].lexical, hits[1].lexical);
    assert_eq!(hits.iter().map(|h| h.rank).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(hits[0].evidence.t_add, r.t);
    assert_eq!(hits[0].evidence.added_at, r.instant);
    // a custom confidence predicate that the hit does not have is absent too
    let works = db.now().encode(&v("works")).unwrap().unwrap();
    let q = TextQuery {
        confidence: Some(works),
        ..TextQuery::new("acme")
    };
    assert!(db
        .now()
        .text_search(&q)
        .unwrap()
        .iter()
        .all(|h| h.evidence.confidence.is_none()));
    // authors of the asserting and confirming transactions
    let opts = TxOptions::default();
    let r = db
        .transact(opts, |tx| {
            let t = tx.t();
            tx.assert(t, Value::iri(vocab::SYS_AUTHOR), v("agent7"), Valid::ALWAYS)?;
            tx.assert(v("cy"), v("works"), Value::str("acme again"), Valid::ALWAYS)?;
            Ok(())
        })
        .unwrap();
    let hit = db
        .now()
        .text_search(&TextQuery::new("again"))
        .unwrap()
        .remove(0);
    assert_eq!(hit.eid, r.asserted[1]);
    assert_eq!(hit.evidence.authors, 1);
}

// @lat: [[tests#Text Retrieval#Equal Hits Keep Their Order]]
#[test]
fn equal_hits_come_back_in_the_same_order_with_the_eid_as_tie_break() {
    let (_d, p) = tmp();
    let db = open(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            for i in 0..6 {
                tx.assert(
                    v(&format!("n{i}")),
                    v("tag"),
                    Value::str("same words here"),
                    Valid::ALWAYS,
                )?;
            }
            Ok(())
        })
        .unwrap();
    let q = TextQuery::new("words");
    let first = db.now().text_search(&q).unwrap();
    assert_eq!(eids(&first), r.asserted); // equal components: eid ascending
    for _ in 0..5 {
        assert_eq!(db.now().text_search(&q).unwrap(), first);
    }
    // the limit keeps the first hits by rank
    let top = TextQuery {
        limit: Some(2),
        ..TextQuery::new("words")
    };
    assert_eq!(eids(&db.now().text_search(&top).unwrap()), r.asserted[..2]);
    assert_eq!(text::RANK_POLICY, "tiramemsu-text-rank/1");
}

/// The bundled host declaring no FTS5 (on the host and on its connections).
struct NoFts(RusqliteHost);

struct NoFtsExec(Box<dyn Executor>);

fn no_fts(c: Capabilities) -> Capabilities {
    Capabilities { fts5: false, ..c }
}

impl Executor for NoFtsExec {
    fn capabilities(&self) -> Capabilities {
        no_fts(self.0.capabilities())
    }
    fn execute(&mut self, sql: &str, params: &[SqlValue]) -> Result<usize> {
        self.0.execute(sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[SqlValue],
        row: &mut dyn FnMut(&[SqlValue]) -> Result<()>,
    ) -> Result<()> {
        self.0.query(sql, params, row)
    }
    fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.0.execute_batch(sql)
    }
    fn begin_immediate(&mut self) -> Result<()> {
        self.0.begin_immediate()
    }
    fn begin_read(&mut self) -> Result<()> {
        self.0.begin_read()
    }
    fn commit(&mut self) -> Result<()> {
        self.0.commit()
    }
    fn rollback(&mut self) -> Result<()> {
        self.0.rollback()
    }
    fn savepoint(&mut self, name: &str) -> Result<()> {
        self.0.savepoint(name)
    }
    fn rollback_to(&mut self, name: &str) -> Result<()> {
        self.0.rollback_to(name)
    }
    fn release(&mut self, name: &str) -> Result<()> {
        self.0.release(name)
    }
    fn registry(&mut self) -> Option<&mut dyn tm_core::HostRegistry> {
        self.0.registry()
    }
}

impl Host for NoFts {
    fn capabilities(&self) -> Capabilities {
        no_fts(self.0.capabilities())
    }
    fn open_writer(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(NoFtsExec(self.0.open_writer(path, opts)?)))
    }
    fn open_reader(&self, path: &Path, opts: &HostOptions) -> Result<Box<dyn Executor>> {
        Ok(Box::new(NoFtsExec(self.0.open_reader(path, opts)?)))
    }
}

fn open_no_fts(p: &Path) -> Db {
    Db::open_with_host(NoFts(RusqliteHost::new()), p, indexed()).unwrap()
}

// @lat: [[tests#Text Retrieval#Host Without FTS5 Rejects Recall]]
#[test]
fn a_host_without_fts5_rejects_recall_and_keeps_ordinary_lookups() {
    let (_d, p) = tmp();
    // `text_index: true` is ignored on this host: the database opens
    let db = open_no_fts(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            tx.assert(
                v("ann"),
                v("note"),
                Value::str("plain lookups still work"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let missing = |e: Result<Vec<TextHit>>| matches!(e, Err(Error::MissingCapability { capability }) if capability == "fts5");
    assert!(missing(db.now().text_search(&TextQuery::new("lookups"))));
    assert!(matches!(
        db.rebuild_text_index(),
        Err(Error::MissingCapability { .. })
    ));
    assert!(matches!(
        db.enable_text_index(),
        Err(Error::MissingCapability { .. })
    ));
    // ordinary lookups, SPARQL and Cypher keep working
    let ann = db.now().encode(&v("ann")).unwrap().unwrap();
    assert_eq!(
        db.now().triples(Some(ann), None, None).unwrap()[0].eid,
        r.asserted[0]
    );
    let s = db
        .now()
        .sparql("SELECT ?o WHERE { v:ann v:note ?o }")
        .unwrap();
    assert_eq!(s.solutions().unwrap().rows.len(), 1);
    let c = db
        .now()
        .cypher(
            "MATCH (n) WHERE n.note IS NOT NULL RETURN n.note",
            &CypherParams::default(),
        )
        .unwrap();
    assert_eq!(c.rows.len(), 1);
    // no FTS5 table was created
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM sqlite_schema WHERE name = 'term_fts'"
        ),
        0
    );
}

// @lat: [[tests#Text Retrieval#Writes Without FTS5 Are Caught Up]]
#[test]
fn strings_written_without_fts5_are_indexed_by_the_next_host_with_it() {
    let (_d, p) = tmp();
    {
        let db = open(&p);
        db.transact(TxOptions::default(), |tx| {
            tx.assert(
                v("ann"),
                v("note"),
                Value::str("first visit"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    }
    let stale_eid;
    {
        // a host without FTS5 writes: it cannot index, so it records the gap
        let db = open_no_fts(&p);
        let r = db
            .transact(TxOptions::default(), |tx| {
                tx.assert(
                    v("bob"),
                    v("note"),
                    Value::str("second visit"),
                    Valid::ALWAYS,
                )?;
                tx.assert(v("cy"), v("note"), Value::str("visit"), Valid::ALWAYS)?;
                Ok(())
            })
            .unwrap();
        stale_eid = r.asserted[0];
        let stale = count(&db, "SELECT value FROM meta WHERE key = 'text_stale'");
        assert_eq!(stale, stale_eid.oid().raw());
    }
    let db = open(&p); // opening with FTS5 indexes the gap
    assert_eq!(
        count(&db, "SELECT value FROM meta WHERE key = 'text_stale'"),
        0
    );
    let hits = db.now().text_search(&TextQuery::new("visit")).unwrap();
    assert_eq!(hits.len(), 3);
    assert!(eids(&hits).contains(&stale_eid));
}

// @lat: [[tests#Text Retrieval#Stale Index Is Refused]]
#[test]
fn a_stale_or_missing_index_is_refused_with_text_index_unavailable() {
    let (_d, p) = tmp();
    let db = Db::open(&p, OpenOptions::default()).unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("ann"),
            v("note"),
            Value::str("never indexed"),
            Valid::ALWAYS,
        )?;
        Ok(())
    })
    .unwrap();
    // the default opens no index: recall says so, and writes keep no index
    assert!(matches!(
        db.now().text_search(&TextQuery::new("indexed")),
        Err(Error::TextIndexUnavailable { .. })
    ));
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM sqlite_schema WHERE name = 'term_fts'"
        ),
        0
    );
    assert!(db.enable_text_index().unwrap()); // built now
    assert!(!db.enable_text_index().unwrap()); // already current
    assert_eq!(
        db.now()
            .text_search(&TextQuery::new("indexed"))
            .unwrap()
            .len(),
        1
    );
    // a gap left by a host without FTS5, seen by a reader before any catch-up
    let stale = "UPDATE meta SET value = 1 WHERE key = 'text_stale'";
    rusqlite::Connection::open(&p)
        .unwrap()
        .execute(stale, [])
        .unwrap();
    assert!(matches!(
        db.now().text_search(&TextQuery::new("indexed")),
        Err(Error::TextIndexUnavailable { .. })
    ));
    // the next write on a host with FTS5 closes the gap
    db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("bob"),
            v("note"),
            Value::str("indexed later"),
            Valid::ALWAYS,
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        db.now()
            .text_search(&TextQuery::new("indexed"))
            .unwrap()
            .len(),
        2
    );
}

// @lat: [[tests#Text Retrieval#Rebuild Restores Recall Without Touching History]]
#[test]
fn rebuild_restores_recall_and_leaves_history_untouched() {
    let (_d, p) = tmp();
    let db = open(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            let a = tx.assert(
                v("ann"),
                v("note"),
                Value::str("quarterly budget review"),
                Valid::ALWAYS,
            )?;
            tx.assert(a.eid(), v("confidence"), Value::Double(0.7), Valid::ALWAYS)?;
            tx.assert(v("bob"), v("note"), Value::str("budget"), Valid::ALWAYS)?;
            tx.assert(
                v("cy"),
                v("note"),
                Value::str("review the budget twice"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    db.transact(TxOptions::default(), |tx| {
        tx.retract(r.asserted[3]).map(|_| ())
    })
    .unwrap();
    let q = TextQuery::new("budget");
    let before = (
        db.now().text_search(&q).unwrap(),
        db.history().text_search(&q).unwrap(),
    );
    let rows = history_rows(&db);
    let events = db.events_since(0).unwrap();
    // damage the derived index, then rebuild it
    rusqlite::Connection::open(&p)
        .unwrap()
        .execute("DELETE FROM term_fts", [])
        .unwrap();
    assert!(db.now().text_search(&q).unwrap().is_empty());
    assert_eq!(db.rebuild_text_index().unwrap(), 3);
    let after = (
        db.now().text_search(&q).unwrap(),
        db.history().text_search(&q).unwrap(),
    );
    assert_eq!(after, before);
    assert_eq!(history_rows(&db), rows);
    assert_eq!(db.events_since(0).unwrap(), events);
    // a second rebuild is a no-op for results too
    db.rebuild_text_index().unwrap();
    assert_eq!(db.now().text_search(&q).unwrap(), before.0);
}

// @lat: [[tests#Text Retrieval#Migration To Format 2 Keeps Every Row]]
#[test]
fn migrating_a_format_1_file_keeps_every_term_statement_and_transaction() {
    use tm_core::storage;
    let (_d, p) = tmp();
    {
        // a file written by a format-1 build
        let exec =
            storage::open_with(&RusqliteHost::new(), &p, &HostOptions::default(), &[], 1).unwrap();
        let mut store = tm_core::Store::from_executor(exec, &p, tm_core::StoreOptions::default());
        store
            .transact(TxOptions::default(), |tx| {
                let e = tx.assert(
                    v("ann"),
                    v("note"),
                    Value::str("written before format two"),
                    Valid::ALWAYS,
                )?;
                tx.assert(v("bob"), v("note"), Value::str("old"), Valid::ALWAYS)?;
                tx.retract(e.eid()).map(|_| ())
            })
            .unwrap();
    }
    let raw_rows = |p: &Path| {
        let raw = rusqlite::Connection::open(p).unwrap();
        let mut out = Vec::new();
        for t in ["triple", "term", "tx"] {
            let mut st = raw
                .prepare(&format!("SELECT * FROM {t} ORDER BY 1"))
                .unwrap();
            let n = st.column_count();
            let rows = st
                .query_map([], |r| {
                    Ok((0..n)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join(","))
                })
                .unwrap();
            out.extend(rows.map(|r| r.unwrap()));
        }
        out
    };
    let version = |p: &Path| -> i64 {
        rusqlite::Connection::open(p)
            .unwrap()
            .query_row(
                "SELECT value FROM meta WHERE key = 'format_version'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(version(&p), 1);
    let before = raw_rows(&p);
    let db = open(&p); // migrates to the current format and builds the index
    assert_eq!(version(&p), storage::FORMAT_VERSION);
    assert_eq!(raw_rows(&p), before);
    assert_eq!(
        db.history()
            .text_search(&TextQuery::new("format"))
            .unwrap()
            .len(),
        1
    );
    assert!(db
        .now()
        .text_search(&TextQuery::new("format"))
        .unwrap()
        .is_empty());
    assert_eq!(
        db.now().text_search(&TextQuery::new("old")).unwrap().len(),
        1
    );
    drop(db);
    // a format-1 build refuses the migrated file instead of letting the index drift
    let r = storage::open_with(&RusqliteHost::new(), &p, &HostOptions::default(), &[], 1);
    assert!(matches!(
        r,
        Err(Error::FormatVersion {
            found: storage::FORMAT_VERSION,
            supported: 1
        })
    ));
}

// @lat: [[tests#Text Retrieval#Recall Sees Speculative Strings]]
#[test]
fn recall_inside_a_speculation_sees_its_strings_and_leaves_no_trace() {
    let (_d, p) = tmp();
    let db = open(&p);
    db.transact(TxOptions::default(), |tx| {
        tx.assert(
            v("ann"),
            v("note"),
            Value::str("committed holiday plan"),
            Valid::ALWAYS,
        )?;
        Ok(())
    })
    .unwrap();
    let seen = db
        .with(
            |tx| {
                tx.assert(
                    v("bob"),
                    v("note"),
                    Value::str("hypothetical holiday"),
                    Valid::ALWAYS,
                )?;
                Ok(())
            },
            |view| Ok(view.text_search(&TextQuery::new("holiday"))?.len()),
        )
        .unwrap();
    assert_eq!(seen, 2);
    assert_eq!(
        db.now()
            .text_search(&TextQuery::new("holiday"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.now()
            .text_search(&TextQuery::new("hypothetical"))
            .unwrap()
            .len(),
        0
    );
    // a dry run is discarded with its index rows too
    db.transact(
        TxOptions {
            dry_run: true,
            ..TxOptions::default()
        },
        |tx| {
            tx.assert(v("cy"), v("note"), Value::str("dry holiday"), Valid::ALWAYS)?;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(count(&db, "SELECT count(*) FROM term_fts"), 1);
}

// @lat: [[tests#Text Retrieval#Recall Honors Budgets]]
#[test]
fn recall_is_one_budgeted_operation() {
    let (_d, p) = tmp();
    let db = open(&p);
    db.transact(TxOptions::default(), |tx| {
        for i in 0..5 {
            tx.assert(
                v(&format!("n{i}")),
                v("note"),
                Value::str("budgeted recall"),
                Valid::ALWAYS,
            )?;
        }
        Ok(())
    })
    .unwrap();
    let q = TextQuery::new("recall");
    let rows = QueryBudget {
        max_rows: Some(3),
        ..Default::default()
    };
    assert!(matches!(
        db.now().with_budget(&rows).text_search(&q),
        Err(Error::ResultLimitExceeded { .. })
    ));
    let limited = TextQuery {
        limit: Some(3),
        ..TextQuery::new("recall")
    };
    assert_eq!(
        db.now()
            .with_budget(&rows)
            .text_search(&limited)
            .unwrap()
            .len(),
        3
    );
    let token = CancelToken::new();
    token.cancel();
    let cancelled = QueryBudget {
        cancel: Some(token),
        ..Default::default()
    };
    assert!(matches!(
        db.now().with_budget(&cancelled).text_search(&q),
        Err(Error::Cancelled)
    ));
    let sparql = "SELECT ?e WHERE { ?e tm:textMatch \"recall\" }";
    assert!(matches!(
        db.now().with_budget(&rows).sparql(sparql),
        Err(Error::ResultLimitExceeded { .. })
    ));
}

fn stmt_iri(e: Eid) -> String {
    format!("urn:tiramemsu:stmt:{}", e.n())
}

// @lat: [[tests#Text Retrieval#SPARQL And Cypher Share The Recall]]
#[test]
fn sparql_and_cypher_run_the_same_recall_as_the_rust_api() {
    let (_d, p) = tmp();
    let db = open(&p);
    let r = db
        .transact(TxOptions::default(), |tx| {
            let a = tx.assert(
                v("ann"),
                v("note"),
                Value::str("project kickoff in lisbon"),
                Valid::ALWAYS,
            )?;
            tx.assert(a.eid(), v("confidence"), Value::Double(0.8), Valid::ALWAYS)?;
            let b = tx.assert(v("bob"), v("note"), Value::str("lisbon"), Valid::ALWAYS)?;
            tx.add_to_graph(b.eid(), v("trip"), AssertOpts::default())?;
            tx.assert(
                v("cy"),
                v("note"),
                Value::str("lisbon lisbon office"),
                Valid::ALWAYS,
            )?;
            Ok(())
        })
        .unwrap();
    let api = db.now().text_search(&TextQuery::new("lisbon")).unwrap();
    assert_eq!(api.len(), 3);
    // SPARQL: the tm:text* group binds the eid, score, rank and confidence
    let q = "SELECT ?e ?score ?rank ?c ?s ?o WHERE { \
               ?e tm:textMatch \"lisbon\" ; tm:textScore ?score ; tm:textRank ?rank ; \
                  tm:textConfidence ?c . \
               ?s ?p ?o ~ ?e } ORDER BY ?rank";
    let res = db.now().sparql(q).unwrap();
    let sol = res.solutions().unwrap();
    assert_eq!(sol.rows.len(), 3);
    let col = |n: &str| sol.vars.iter().position(|x| x == n).unwrap();
    for (row, hit) in sol.rows.iter().zip(&api) {
        assert_eq!(row[col("e")], Some(Value::Stmt(hit.eid)));
        assert_eq!(row[col("score")], Some(Value::Double(hit.lexical)));
        assert_eq!(row[col("rank")], Some(Value::Int(hit.rank as i64)));
        assert_eq!(row[col("c")], hit.evidence.confidence.map(Value::Double));
        assert_eq!(row[col("o")], Some(Value::str(hit.text.clone())));
    }
    // limit, GRAPH and SERVICE time scopes apply to the recall
    let top = db
        .now()
        .sparql("SELECT ?e WHERE { ?e tm:textMatch \"lisbon\" ; tm:textLimit 1 }")
        .unwrap();
    assert_eq!(
        top.solutions().unwrap().rows,
        vec![vec![Some(Value::Stmt(api[0].eid))]]
    );
    let in_trip = db
        .now()
        .sparql("SELECT ?e WHERE { GRAPH v:trip { ?e tm:textMatch \"lisbon\" } }")
        .unwrap();
    assert_eq!(
        in_trip.solutions().unwrap().rows,
        vec![vec![Some(Value::Stmt(r.asserted[2]))]]
    );
    let before = db
        .now()
        .sparql(
            "SELECT ?e WHERE { SERVICE <urn:tiramemsu:tm:asOf/0> { ?e tm:textMatch \"lisbon\" } }",
        )
        .unwrap();
    assert!(before.solutions().unwrap().rows.is_empty());
    let any = db
        .now()
        .sparql("SELECT ?e WHERE { ?e tm:textMatch \"office nowhere\" ; tm:textMode \"any\" }")
        .unwrap();
    assert_eq!(any.solutions().unwrap().rows.len(), 1);
    // misuse is reported before anything runs
    for bad in [
        "SELECT ?e WHERE { ?e tm:textScore ?s }",
        "SELECT ?e WHERE { ?e tm:textMatch ?q }",
        "SELECT ?e WHERE { v:x tm:textMatch \"a\" }",
        "SELECT ?e WHERE { GRAPH ?g { ?e tm:textMatch \"a\" } }",
        "SELECT ?e WHERE { ?e tm:textMatch \"a\" ; tm:textMode \"near\" }",
    ] {
        assert!(
            matches!(db.now().sparql(bad), Err(Error::InvalidQuery { .. })),
            "{bad}"
        );
    }
    // Cypher: the procedure yields the same hits in the same order
    let none = CypherParams::default();
    let c = db
        .now()
        .cypher(
            "CALL tiramemsu.text.search('lisbon') YIELD statement, score, rank, confidence, text \
             RETURN statement, score, rank, confidence, text",
            &none,
        )
        .unwrap();
    assert_eq!(c.rows.len(), 3);
    for (row, hit) in c.rows.iter().zip(&api) {
        match &row[0] {
            CypherValue::Node(n) => assert_eq!(n.element_id, stmt_iri(hit.eid)),
            other => panic!("statement is {other:?}"),
        }
        assert_eq!(row[1], CypherValue::Float(hit.lexical));
        assert_eq!(row[2], CypherValue::Integer(hit.rank as i64));
        assert_eq!(
            row[3],
            hit.evidence
                .confidence
                .map_or(CypherValue::Null, CypherValue::Float)
        );
        assert_eq!(row[4], CypherValue::String(hit.text.clone()));
    }
    let opts = db
        .now()
        .cypher(
            "CALL tiramemsu.text.search('lisbon', {limit: 5, graphs: ['trip']}) YIELD subject, predicate \
             RETURN subject, predicate",
            &none,
        )
        .unwrap();
    assert_eq!(opts.rows.len(), 1);
    assert_eq!(opts.rows[0][1], CypherValue::String("note".into()));
    // standalone CALL returns every column
    let all = db
        .now()
        .cypher("CALL tiramemsu.text.search('kickoff')", &none)
        .unwrap();
    assert_eq!(
        all.columns,
        [
            "statement",
            "subject",
            "predicate",
            "text",
            "score",
            "rank",
            "confidence"
        ]
    );
    assert_eq!(all.rows.len(), 1);
    // a time clause applies to the recall
    let past = db
        .now()
        .cypher(
            "USE AS OF 0 CALL tiramemsu.text.search('lisbon') YIELD statement RETURN statement",
            &none,
        )
        .unwrap();
    assert!(past.rows.is_empty());
}
