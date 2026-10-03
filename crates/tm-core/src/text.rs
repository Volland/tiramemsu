//! Text recall over string objects with evidence ranking (OpenSpec change
//! `add-text-retrieval`).
//!
//! The index is derived storage: the FTS5 table `term_fts` holds one row per
//! distinct string value ever stored as a statement object, keyed by the value's
//! full ObjectId (`rowid`). Dictionary strings (`STR`, `LANG_STR`) and inline short
//! strings (`SHORT_STR`) are both indexed; an inline string is decoded into the
//! index and never gets a dictionary row. Typed literals, IRIs and numbers are not
//! searchable.
//!
//! A recall matches the index first and then joins the matching values to `triple`
//! through [`scan_predicates`], the one function that writes time predicates, so a
//! retracted statement is absent from a now recall exactly as it is absent from a
//! now scan. Every query of one recall runs on the caller's connection, in its
//! snapshot.
//!
//! The index is kept current inside each write transaction (so a speculation or a
//! transaction sees its own strings), only on hosts that declare `fts5`. A host
//! without FTS5 never issues FTS5 SQL: it records in `meta.text_stale` that it
//! wrote strings the index lacks, and the next writer with FTS5 indexes them.

use std::cmp::Ordering;

use crate::budget;
use crate::codec;
use crate::error::{Error, Result};
use crate::exec::{Executor, Params, SqlValue};
use crate::id::{Eid, ObjectId, Tag, TxId};
use crate::term::TermReader;
use crate::value::Value;
use crate::view::{scan_predicates, ViewSpec};
use crate::vocab;

/// The version of the index layout this build writes: the FTS5 columns
/// `(text, lang UNINDEXED)` and the tokenizer [`TOKENIZER`]. Stored in
/// `meta.text_index`; a different stored version is rebuilt when the index is
/// enabled.
pub const TEXT_INDEX_VERSION: i64 = 1;

/// The FTS5 tokenizer of index version 1: Unicode 6.1 word boundaries, case
/// folding and diacritics removed (`"Zoë"` matches `zoe`).
pub const TOKENIZER: &str = "unicode61 remove_diacritics 2";

/// The name of the ranking policy [`search`] applies; see [`TextHit`] for the order.
pub const RANK_POLICY: &str = "tiramemsu-text-rank/1";

/// The predicate read as a confidence layer when [`TextQuery::confidence`] is
/// `None`: `v:confidence` in the default vocabulary.
pub const DEFAULT_CONFIDENCE: &str = "urn:tiramemsu:v:confidence";

const DDL: &str = "CREATE VIRTUAL TABLE term_fts USING fts5(text, lang UNINDEXED, \
                   tokenize = 'unicode61 remove_diacritics 2')";

/// How the words of [`TextQuery::text`] combine.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextMode {
    /// Every word must occur (the default).
    #[default]
    All,
    /// At least one word must occur.
    Any,
    /// The words must occur next to each other, in order.
    Phrase,
}

impl TextMode {
    /// The lower-case name used by the query languages and the JSON bridge.
    pub fn name(self) -> &'static str {
        match self {
            TextMode::All => "all",
            TextMode::Any => "any",
            TextMode::Phrase => "phrase",
        }
    }

    /// Inverse of [`TextMode::name`] (case-insensitive).
    pub fn from_name(s: &str) -> Option<TextMode> {
        match s.to_ascii_lowercase().as_str() {
            "all" => Some(TextMode::All),
            "any" => Some(TextMode::Any),
            "phrase" => Some(TextMode::Phrase),
            _ => None,
        }
    }
}

/// One text recall: the words, how they combine, and the statement filters.
/// `TextQuery::new(text)` is an all-words recall with no filter and no limit.
///
/// Words are split on whitespace and matched as whole tokens after case folding
/// and diacritics removal; a word ending in `*` matches as a prefix (`acm*`).
/// Query text is never interpreted as FTS5 syntax.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextQuery {
    /// The words to recall.
    pub text: String,
    /// How the words combine.
    pub mode: TextMode,
    /// Only statements that are members of at least one of these graphs, the
    /// membership being visible in the view. `None` = every statement;
    /// `Some(vec![])` matches nothing.
    pub graphs: Option<Vec<ObjectId>>,
    /// Only statements with one of these predicates. `None` = any predicate.
    pub predicates: Option<Vec<ObjectId>>,
    /// At most this many hits, after ranking. `None` = all.
    pub limit: Option<usize>,
    /// The predicate whose numeric objects on a hit's eid are its confidence
    /// layer. `None` = [`DEFAULT_CONFIDENCE`].
    pub confidence: Option<ObjectId>,
}

impl TextQuery {
    /// An all-words recall of `text` with no filter and no limit.
    pub fn new(text: impl Into<String>) -> TextQuery {
        TextQuery {
            text: text.into(),
            ..TextQuery::default()
        }
    }
}

/// The evidence the store holds about one hit, read in the recall's view. A layer
/// the statement does not have is reported as absent (`None`) or as a zero count,
/// never estimated.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEvidence {
    /// The largest numeric object of the hit's visible confidence statements
    /// (`INT`, `DOUBLE` or `DECIMAL`); `None` when it has none.
    pub confidence: Option<f64>,
    /// Visible `sys:confirmedBy` statements on the hit.
    pub confirmations: u64,
    /// Distinct `sys:author` values of the transaction that asserted the hit and
    /// of the transactions that confirmed it.
    pub authors: u64,
    /// The transaction that asserted the hit.
    pub t_add: TxId,
    /// That transaction's instant (epoch ms).
    pub added_at: i64,
}

/// One recalled statement with its lexical score and evidence.
///
/// Hits are ordered by the policy [`RANK_POLICY`]: lexical score descending, then
/// confidence descending (absent last), confirmations descending, authors
/// descending, `added_at` descending (newer first), and statement eid ascending as
/// the final tie-break, so equal hits always come back in the same order.
#[derive(Clone, Debug, PartialEq)]
pub struct TextHit {
    /// The statement.
    pub eid: Eid,
    /// Its subject.
    pub s: ObjectId,
    /// Its predicate.
    pub p: ObjectId,
    /// Its object: the matched string.
    pub o: ObjectId,
    /// The matched text.
    pub text: String,
    /// The language tag of a language-tagged string, lower-cased.
    pub lang: Option<String>,
    /// The lexical relevance: FTS5 `bm25` negated, so larger is better. Equal
    /// strings score equally.
    pub lexical: f64,
    /// 1-based position under the ranking policy, before the limit.
    pub rank: u64,
    /// The evidence components.
    pub evidence: TextEvidence,
}

/// The persistent state of the text index (`meta`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TextState {
    /// The stored index version; 0 when the index was never built.
    pub version: i64,
    /// The lowest eid (raw ObjectId) of a string statement written by a host
    /// without FTS5 since the index was last brought up to date; 0 when none.
    pub stale_from: i64,
}

impl TextState {
    /// True when the index exists.
    pub fn built(&self) -> bool {
        self.version > 0
    }
}

/// Reads the index state. A file without the format-2 rows reads as not built.
pub fn state(exec: &mut dyn Executor) -> Result<TextState> {
    let mut st = TextState::default();
    exec.query(
        "SELECT key, value FROM meta WHERE key IN ('text_index', 'text_stale')",
        &[],
        &mut |r| {
            let v = r[1].as_i64().unwrap_or(0);
            match r[0].as_str() {
                Some("text_index") => st.version = v,
                Some("text_stale") => st.stale_from = v,
                _ => {}
            }
            Ok(())
        },
    )?;
    Ok(st)
}

fn set_meta(exec: &mut dyn Executor, key: &str, value: i64) -> Result<()> {
    exec.execute(
        "UPDATE meta SET value = ?1 WHERE key = ?2",
        &[SqlValue::Integer(value), SqlValue::from(key)],
    )?;
    Ok(())
}

fn require_fts5(exec: &dyn Executor) -> Result<()> {
    if exec.capabilities().fts5 {
        Ok(())
    } else {
        Err(Error::MissingCapability {
            capability: "fts5".to_string(),
        })
    }
}

/// True for the tags whose values are searchable text.
pub fn is_text(id: ObjectId) -> bool {
    matches!(id.tag(), Ok(Tag::ShortStr | Tag::Str | Tag::LangStr))
}

/// Indexes one string value if the index does not hold it yet. `text` and `lang`
/// are its decoded form.
pub(crate) fn index_value(
    exec: &mut dyn Executor,
    id: ObjectId,
    text: &str,
    lang: Option<&str>,
) -> Result<()> {
    exec.execute(
        "INSERT INTO term_fts(rowid, text, lang) SELECT ?1, ?2, ?3 \
         WHERE NOT EXISTS (SELECT 1 FROM term_fts WHERE rowid = ?1)",
        &[
            SqlValue::Integer(id.raw()),
            SqlValue::from(text),
            lang.map_or(SqlValue::Null, SqlValue::from),
        ],
    )?;
    Ok(())
}

/// Indexes the string objects of every statement with `eid >= from` (raw), the
/// dictionary ones in one statement and the inline ones decoded here.
fn index_statements_from(exec: &mut dyn Executor, from: i64) -> Result<u64> {
    let before = count(exec)?;
    exec.execute(
        "INSERT INTO term_fts(rowid, text, lang) \
         SELECT x.o, m.lex, m.lang FROM \
           (SELECT DISTINCT o FROM triple WHERE eid >= ?1 AND (o & 15) IN (10, 11)) AS x \
         JOIN term AS m ON m.id = (x.o >> 4) \
         WHERE NOT EXISTS (SELECT 1 FROM term_fts WHERE rowid = x.o)",
        &[SqlValue::Integer(from)],
    )?;
    let inline = exec.rows(
        "SELECT DISTINCT o FROM triple WHERE eid >= ?1 AND (o & 15) = 9",
        &[SqlValue::Integer(from)],
    )?;
    for r in inline {
        let id = ObjectId::from_raw(r[0].as_i64().unwrap_or(0));
        if let Some(Value::Str(s)) = codec::decode_inline(id)? {
            index_value(exec, id, &s, None)?;
        }
    }
    Ok(count(exec)? - before)
}

fn count(exec: &mut dyn Executor) -> Result<u64> {
    Ok(exec
        .query_i64("SELECT count(*) FROM term_fts", &[])?
        .unwrap_or(0) as u64)
}

fn table_exists(exec: &mut dyn Executor) -> Result<bool> {
    Ok(exec
        .query_i64(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'term_fts'",
            &[],
        )?
        .unwrap_or(0)
        > 0)
}

/// Drops and rebuilds the index from the statements (live and retracted), then
/// records it as current. Runs inside the caller's write transaction; reads and
/// changes no `triple`, `term` or `tx` row. Returns the number of indexed values.
///
/// # Errors
///
/// `MissingCapability("fts5")` on a host without FTS5, before anything changes.
// @lat: [[storage#Text Index]]
pub fn rebuild(exec: &mut dyn Executor) -> Result<u64> {
    require_fts5(exec)?;
    exec.execute_batch("DROP TABLE IF EXISTS term_fts")?;
    exec.execute_batch(DDL)?;
    let n = index_statements_from(exec, 0)?;
    set_meta(exec, "text_index", TEXT_INDEX_VERSION)?;
    set_meta(exec, "text_stale", 0)?;
    Ok(n)
}

/// Builds the index unless it is already current; brings a stale one up to date.
/// Returns true when it (re)built the whole index.
///
/// # Errors
///
/// As [`rebuild`].
pub fn enable(exec: &mut dyn Executor) -> Result<bool> {
    require_fts5(exec)?;
    let st = state(exec)?;
    if st.version != TEXT_INDEX_VERSION || !table_exists(exec)? {
        rebuild(exec)?;
        return Ok(true);
    }
    catch_up(exec, st)?;
    Ok(false)
}

/// Indexes what a host without FTS5 wrote since `st.stale_from` (no-op when the
/// index is current or not built, or the host lacks FTS5).
pub fn catch_up(exec: &mut dyn Executor, st: TextState) -> Result<()> {
    if !st.built() || st.stale_from == 0 || !exec.capabilities().fts5 {
        return Ok(());
    }
    index_statements_from(exec, st.stale_from)?;
    set_meta(exec, "text_stale", 0)
}

/// How a write transaction keeps the index (decided at its start).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Upkeep {
    /// No index: nothing to do.
    #[default]
    Off,
    /// Index each new string object.
    Index,
    /// The host lacks FTS5: record the first string statement in `text_stale`.
    Defer {
        /// Already recorded in this transaction.
        marked: bool,
    },
}

impl Upkeep {
    /// Reads the state at the start of a write transaction and, on a host with
    /// FTS5, first indexes what a host without it left behind.
    pub(crate) fn begin(exec: &mut dyn Executor) -> Result<Upkeep> {
        let st = state(exec)?;
        if !st.built() {
            return Ok(Upkeep::Off);
        }
        if !exec.capabilities().fts5 {
            return Ok(Upkeep::Defer {
                marked: st.stale_from != 0,
            });
        }
        catch_up(exec, st)?;
        Ok(Upkeep::Index)
    }
}

// ---------------------------------------------------------------------------
// Recall

/// The FTS5 match expression of `q`: every word quoted, so query text is never
/// FTS5 syntax. `None` when the text has no word.
pub fn match_expression(text: &str, mode: TextMode) -> Option<String> {
    let quote = |w: &str| format!("\"{}\"", w.replace('"', "\"\""));
    let mut words = Vec::new();
    for w in text.split_whitespace() {
        let (body, prefix) = match w.strip_suffix('*') {
            Some(b) if mode != TextMode::Phrase => (b, true),
            _ => (w, false),
        };
        let body = body.trim_matches('*');
        if body.chars().any(char::is_alphanumeric) {
            words.push((body.to_string(), prefix));
        }
    }
    if words.is_empty() {
        return None;
    }
    Some(match mode {
        TextMode::Phrase => quote(
            &words
                .iter()
                .map(|(w, _)| w.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        TextMode::All | TextMode::Any => {
            let parts: Vec<String> = words
                .iter()
                .map(|(w, p)| format!("{}{}", quote(w), if *p { "*" } else { "" }))
                .collect();
            parts.join(if mode == TextMode::All { " " } else { " OR " })
        }
    })
}

/// Checks that recall can run on this connection: the host has FTS5, the index
/// exists and no host without FTS5 wrote strings it lacks.
pub fn check_available(exec: &mut dyn Executor) -> Result<()> {
    require_fts5(exec)?;
    let st = state(exec)?;
    if !st.built() {
        return Err(Error::TextIndexUnavailable {
            reason: "the text index was never built (open with `text_index` or rebuild it)"
                .to_string(),
        });
    }
    if st.version != TEXT_INDEX_VERSION {
        return Err(Error::TextIndexUnavailable {
            reason: format!(
                "the text index has version {}, this build reads {TEXT_INDEX_VERSION}; rebuild it",
                st.version
            ),
        });
    }
    if st.stale_from != 0 {
        return Err(Error::TextIndexUnavailable {
            reason: "a host without FTS5 wrote strings the index lacks; the next write on a \
                     host with FTS5, or a rebuild, indexes them"
                .to_string(),
        });
    }
    Ok(())
}

/// Recalls the statements selected by `spec` whose object matches `q`, ranked by
/// [`RANK_POLICY`]. Runs every query on `exec`, so call it inside one read
/// transaction (or on the writer of a speculation, which sees its own strings).
/// `terms` decodes confidence values; pass `use_cache = false` on uncommitted state.
///
/// Polls the operation budget once per candidate and charges the returned hits.
///
/// # Errors
///
/// `MissingCapability("fts5")` on a host without FTS5, `TextIndexUnavailable` when
/// the index was never built or is behind, `InvalidQuery` for text without a word.
// @lat: [[query#Text Recall]]
pub fn search(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    q: &TextQuery,
    terms: &TermReader,
    use_cache: bool,
) -> Result<Vec<TextHit>> {
    check_available(exec)?;
    let Some(expr) = match_expression(&q.text, q.mode) else {
        return Err(Error::invalid_query(format!(
            "text recall needs at least one word, got {:?}",
            q.text
        )));
    };
    if matches!(&q.graphs, Some(g) if g.is_empty())
        || matches!(&q.predicates, Some(p) if p.is_empty())
    {
        return Ok(Vec::new());
    }
    let mut params = Params::new();
    let m = params.push(SqlValue::Text(expr));
    let mut conds = vec![format!("t.o = m.oid")];
    let time = scan_predicates(spec, "t", &mut params);
    if !time.is_empty() {
        conds.push(time);
    }
    if let Some(ps) = &q.predicates {
        let list: Vec<String> = ps.iter().map(|p| params.push(p.raw())).collect();
        conds.push(format!("t.p IN ({})", list.join(", ")));
    }
    if let Some(gs) = &q.graphs {
        let Some(ig) = TermReader::encode(exec, &Value::iri(vocab::SYS_IN_GRAPH))? else {
            // no membership was ever written
            return Ok(Vec::new());
        };
        let ig = params.push(ig.raw());
        let list: Vec<String> = gs.iter().map(|g| params.push(g.raw())).collect();
        let mut member = vec![
            "g.s = t.eid".to_string(),
            format!("g.p = {ig}"),
            format!("g.o IN ({})", list.join(", ")),
        ];
        let gt = scan_predicates(spec, "g", &mut params);
        if !gt.is_empty() {
            member.push(gt);
        }
        conds.push(format!(
            "EXISTS (SELECT 1 FROM triple AS g WHERE {})",
            member.join(" AND ")
        ));
    }
    let sql = format!(
        "WITH m AS MATERIALIZED (SELECT rowid AS oid, text, lang, bm25(term_fts) AS bm \
         FROM term_fts WHERE term_fts MATCH {m}) \
         SELECT t.eid, t.s, t.p, t.o, t.t_add, m.text, m.lang, m.bm, \
           (SELECT instant FROM tx WHERE tx.t = t.t_add) \
         FROM m JOIN triple AS t ON {} ORDER BY t.eid",
        conds.join(" AND ")
    );
    let mut hits = Vec::new();
    exec.query(&sql, params.values(), &mut |r| {
        let i = |k: usize| r[k].as_i64().unwrap_or(0);
        hits.push(TextHit {
            eid: Eid::from_oid(ObjectId::from_raw(i(0))).unwrap_or(Eid::new(0)),
            s: ObjectId::from_raw(i(1)),
            p: ObjectId::from_raw(i(2)),
            o: ObjectId::from_raw(i(3)),
            text: r[5].as_str().unwrap_or_default().to_string(),
            lang: r[6].as_str().map(str::to_string),
            lexical: -r[7].as_f64().unwrap_or(0.0),
            rank: 0,
            evidence: TextEvidence {
                confidence: None,
                confirmations: 0,
                authors: 0,
                t_add: TxId(i(4) as u64),
                added_at: i(8),
            },
        });
        Ok(())
    })?;
    let ev = EvidenceReader::new(exec, spec, q.confidence)?;
    for h in &mut hits {
        budget::check()?;
        ev.fill(exec, h, terms, use_cache)?;
    }
    hits.sort_by(compare);
    for (i, h) in hits.iter_mut().enumerate() {
        h.rank = i as u64 + 1;
    }
    if let Some(l) = q.limit {
        hits.truncate(l);
    }
    if budget::active() {
        budget::charge_rows(hits.len() as u64)?;
        let bytes: u64 = hits.iter().map(|h| 96 + h.text.len() as u64).sum();
        budget::charge_bytes(bytes)?;
    }
    Ok(hits)
}

/// The order of [`RANK_POLICY`].
pub fn compare(a: &TextHit, b: &TextHit) -> Ordering {
    b.lexical
        .total_cmp(&a.lexical)
        .then_with(|| match (a.evidence.confidence, b.evidence.confidence) {
            (Some(x), Some(y)) => y.total_cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| b.evidence.confirmations.cmp(&a.evidence.confirmations))
        .then_with(|| b.evidence.authors.cmp(&a.evidence.authors))
        .then_with(|| b.evidence.added_at.cmp(&a.evidence.added_at))
        .then_with(|| a.eid.cmp(&b.eid))
}

/// The evidence queries of one recall (and of conflict inspection), with the
/// view's time predicates. A layer predicate that is not interned is `None`.
pub(crate) struct EvidenceReader {
    spec: ViewSpec,
    confidence: Option<ObjectId>,
    pub(crate) confirmed_by: Option<ObjectId>,
    pub(crate) author: Option<ObjectId>,
}

impl EvidenceReader {
    /// The reader of `spec`, with `confidence` as the confidence layer
    /// ([`DEFAULT_CONFIDENCE`] when `None`).
    pub(crate) fn new(
        exec: &mut dyn Executor,
        spec: &ViewSpec,
        confidence: Option<ObjectId>,
    ) -> Result<EvidenceReader> {
        let confidence = match confidence {
            Some(c) => Some(c),
            None => TermReader::encode(exec, &Value::iri(DEFAULT_CONFIDENCE))?,
        };
        Ok(EvidenceReader {
            spec: *spec,
            confidence,
            confirmed_by: TermReader::encode(exec, &Value::iri(vocab::SYS_CONFIRMED_BY))?,
            author: TermReader::encode(exec, &Value::iri(vocab::SYS_AUTHOR))?,
        })
    }

    /// The objects of the visible `(eid, p, ?)` statements.
    pub(crate) fn objects(
        &self,
        exec: &mut dyn Executor,
        s: ObjectId,
        p: ObjectId,
    ) -> Result<Vec<ObjectId>> {
        let mut params = Params::new();
        let sp = params.push(s.raw());
        let pp = params.push(p.raw());
        let mut sql = format!("SELECT a.o FROM triple AS a WHERE a.s = {sp} AND a.p = {pp}");
        let time = scan_predicates(&self.spec, "a", &mut params);
        if !time.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&time);
        }
        sql.push_str(" ORDER BY a.eid");
        Ok(exec
            .rows(&sql, params.values())?
            .into_iter()
            .map(|r| ObjectId::from_raw(r[0].as_i64().unwrap_or(0)))
            .collect())
    }

    /// The largest numeric object of the visible confidence statements on `me`
    /// (`INT`, `DOUBLE` or `DECIMAL`); `None` when it has none.
    pub(crate) fn confidence(
        &self,
        exec: &mut dyn Executor,
        me: ObjectId,
        terms: &TermReader,
        use_cache: bool,
    ) -> Result<Option<f64>> {
        let Some(c) = self.confidence else {
            return Ok(None);
        };
        let mut best: Option<f64> = None;
        for o in self.objects(exec, me, c)? {
            let x = match terms.decode(exec, o, use_cache)? {
                Value::Int(i) => Some(i as f64),
                Value::Double(d) if !d.is_nan() => Some(d),
                Value::Decimal(s) => s.parse::<f64>().ok(),
                _ => None,
            };
            if let Some(x) = x {
                best = Some(best.map_or(x, |b: f64| b.max(x)));
            }
        }
        Ok(best)
    }

    fn fill(
        &self,
        exec: &mut dyn Executor,
        h: &mut TextHit,
        terms: &TermReader,
        use_cache: bool,
    ) -> Result<()> {
        let me = h.eid.oid();
        h.evidence.confidence = self.confidence(exec, me, terms, use_cache)?;
        let mut txs = vec![h.evidence.t_add.oid()];
        if let Some(cb) = self.confirmed_by {
            let confirmations = self.objects(exec, me, cb)?;
            h.evidence.confirmations = confirmations.len() as u64;
            txs.extend(
                confirmations
                    .into_iter()
                    .filter(|o| o.tag_bits() == Tag::Tx as u8),
            );
        }
        if let Some(author) = self.author {
            let mut seen = std::collections::BTreeSet::new();
            for t in txs {
                seen.extend(self.objects(exec, t, author)?);
            }
            h.evidence.authors = seen.len() as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_expressions_quote_every_word() {
        let m = |t: &str, mode| match_expression(t, mode);
        assert_eq!(
            m("alice acme", TextMode::All).unwrap(),
            "\"alice\" \"acme\""
        );
        assert_eq!(
            m("alice acme", TextMode::Any).unwrap(),
            "\"alice\" OR \"acme\""
        );
        assert_eq!(m("alice acme", TextMode::Phrase).unwrap(), "\"alice acme\"");
        assert_eq!(m("acm*", TextMode::All).unwrap(), "\"acm\"*");
        assert_eq!(
            m("say \"hi\"", TextMode::All).unwrap(),
            "\"say\" \"\"\"hi\"\"\""
        );
        assert_eq!(
            m("NEAR(a b) OR", TextMode::All).unwrap(),
            "\"NEAR(a\" \"b)\" \"OR\""
        );
        assert!(m("  ", TextMode::All).is_none());
        assert!(m("* -- ", TextMode::All).is_none());
    }

    #[test]
    fn modes_round_trip() {
        for m in [TextMode::All, TextMode::Any, TextMode::Phrase] {
            assert_eq!(TextMode::from_name(m.name()), Some(m));
        }
        assert_eq!(TextMode::from_name("PHRASE"), Some(TextMode::Phrase));
        assert_eq!(TextMode::from_name("near"), None);
    }
}
