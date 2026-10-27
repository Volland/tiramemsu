//! The text index over the storage traits: an FTS5-compatible tokeniser and the
//! upkeep the engine runs for every new string object.
//!
//! Tokens follow SQLite FTS5's `unicode61 remove_diacritics 2`: runs of letters
//! (`L*`), numbers (`N*`) and private-use characters (`Co`), case-folded, with
//! diacritics removed. Combining marks inside a run belong to it and disappear
//! with the diacritic removal. Positions count tokens from 0, as FTS5 does.

use tm_core::text::{is_text, TEXT_INDEX_VERSION};
use tm_core::{ObjectId, Result, Value};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::dict::TermCache;
use crate::store::{Pattern, ReadStore, Scope, TextDoc, WriteStore};

/// True for a character FTS5's default `unicode61` categories treat as part of a
/// token (letters, numbers, private use).
fn token_char(c: char) -> bool {
    c.is_alphanumeric() || ('\u{E000}'..='\u{F8FF}').contains(&c)
}

/// True for the Latin blocks whose diacritics FTS5's `remove_diacritics` folds.
fn latin(c: char) -> bool {
    matches!(c, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

/// Case-folds one token and removes the diacritics of its Latin letters; a
/// combining mark is dropped when it follows a Latin letter or stands alone.
fn fold(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut after_latin = true;
    for c in token.chars().flat_map(char::to_lowercase) {
        if latin(c) {
            out.extend(std::iter::once(c).nfd().filter(|d| !is_combining_mark(*d)));
            after_latin = true;
        } else if is_combining_mark(c) {
            if !after_latin {
                out.push(c);
            }
        } else {
            out.push(c);
            after_latin = c.is_ascii();
        }
    }
    out
}

/// Splits `text` into folded tokens in order.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if token_char(c) || (!cur.is_empty() && is_combining_mark(c)) {
            cur.push(c);
        } else if !cur.is_empty() {
            let t = fold(&cur);
            if !t.is_empty() {
                out.push(t);
            }
            cur.clear();
        }
    }
    if !cur.is_empty() {
        let t = fold(&cur);
        if !t.is_empty() {
            out.push(t);
        }
    }
    out
}

/// The distinct tokens of `text` with their positions, ascending by token.
pub fn tokenize(text: &str) -> (u32, Vec<(String, Vec<u32>)>) {
    let toks = tokens(text);
    let mut map: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
    for (i, t) in toks.iter().enumerate() {
        map.entry(t.clone()).or_default().push(i as u32);
    }
    (toks.len() as u32, map.into_iter().collect())
}

/// How a write transaction keeps the index (decided at its start).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Upkeep {
    /// No index: nothing to do.
    #[default]
    Off,
    /// Index each new string object.
    Index,
}

impl Upkeep {
    /// Reads the index state at the start of a write transaction.
    pub fn begin(store: &dyn ReadStore) -> Result<Upkeep> {
        Ok(if store.meta("text_index")?.unwrap_or(0) > 0 {
            Upkeep::Index
        } else {
            Upkeep::Off
        })
    }
}

/// Indexes one string value unless the index holds it.
pub fn index_value(
    store: &mut dyn WriteStore,
    id: ObjectId,
    text: &str,
    lang: Option<&str>,
) -> Result<()> {
    let (len, toks) = tokenize(text);
    store.text_insert(
        id.raw(),
        &TextDoc {
            len,
            lang: lang.map(str::to_string),
        },
        &toks,
    )?;
    Ok(())
}

/// Drops and refills the index from every statement, live and retracted;
/// returns the number of indexed values.
pub fn rebuild(store: &mut dyn WriteStore) -> Result<u64> {
    store.text_clear()?;
    let terms = TermCache::new(1024);
    let mut values: Vec<i64> = Vec::new();
    for r in store.scan(Pattern::ANY, Scope::All)? {
        let r = r?;
        if is_text(ObjectId::from_raw(r.o)) {
            values.push(r.o);
        }
    }
    values.sort_unstable();
    values.dedup();
    let mut n = 0;
    for v in values {
        let id = ObjectId::from_raw(v);
        let (body, lang) = match terms.decode(&*store, id, false)? {
            Value::Str(s) => (s, None),
            Value::LangStr { lex, lang } => (lex, Some(lang)),
            _ => continue,
        };
        index_value(store, id, &body, lang.as_deref())?;
        n += 1;
    }
    store.set_meta("text_index", TEXT_INDEX_VERSION)?;
    store.set_meta("text_stale", 0)?;
    Ok(n)
}

/// Builds the index unless it is built; returns true when it built it.
pub fn enable(store: &mut dyn WriteStore) -> Result<bool> {
    if store.meta("text_index")?.unwrap_or(0) == TEXT_INDEX_VERSION {
        return Ok(false);
    }
    rebuild(store)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_fold_case_and_diacritics() {
        assert_eq!(
            tokens("Crème Brûlée, naïve-CAFÉ 42x"),
            ["creme", "brulee", "naive", "cafe", "42x"]
        );
        assert_eq!(tokens("  "), Vec::<String>::new());
        assert_eq!(tokens("Київ і Львів"), ["київ", "і", "львів"]);
    }

    #[test]
    fn positions_count_tokens() {
        let (n, t) = tokenize("a b a");
        assert_eq!(n, 3);
        assert_eq!(
            t,
            vec![("a".to_string(), vec![0, 2]), ("b".to_string(), vec![1])]
        );
    }
}
