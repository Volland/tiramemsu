//! Name to IRI mapping (`lat.md/data-model#Vocabulary Mapping`): resolution of
//! labels, types, keys and path atoms through `@vocab` and the prefix table, and
//! rendering of IRIs back to names. Shared by the Cypher front end and the path
//! engine.

use crate::error::Result;
use crate::exec::Executor;
use crate::read;
use crate::term::TermReader;
use crate::value::Value;
use crate::view::ViewSpec;
use crate::vocab::{self, RDF, SYS, TM, V, XSD};

/// The `rdfs:` namespace.
pub const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";

/// Built-in prefix names; `v` follows the current `@vocab`.
pub const BUILTIN_PREFIXES: [&str; 6] = ["sys", "tm", "rdf", "rdfs", "xsd", "v"];

/// The vocabulary configuration current at compile time.
#[derive(Clone, Debug, PartialEq)]
pub struct Vocab {
    /// The `@vocab` base.
    pub vocab: String,
    /// User prefixes `(name, iri)`.
    pub prefixes: Vec<(String, String)>,
}

impl Default for Vocab {
    fn default() -> Vocab {
        Vocab {
            vocab: V.to_string(),
            prefixes: Vec::new(),
        }
    }
}

/// Percent-encodes what is not allowed in an IRI (ASCII controls, space, `"<>\^`{|}`, `%`).
pub fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let bad = ch.is_ascii()
            && (ch.is_ascii_control()
                || matches!(
                    ch,
                    ' ' | '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}' | '%'
                ));
        if bad {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Decodes `%XX` sequences (invalid ones are kept).
pub fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(Ok(v)) = s.get(i + 1..i + 3).map(|h| u8::from_str_radix(h, 16)) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// True for `scheme:rest` with a syntactically valid scheme and no forbidden characters.
pub fn is_absolute_iri(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    let mut cs = scheme.chars();
    let ok_scheme = matches!(cs.next(), Some(c) if c.is_ascii_alphabetic())
        && cs.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok_scheme
        && !rest.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`')
        })
}

impl Vocab {
    /// The IRI of a declared prefix (built-ins included).
    pub fn prefix_iri(&self, name: &str) -> Option<String> {
        Some(match name {
            "sys" => SYS.to_string(),
            "tm" => TM.to_string(),
            "rdf" => RDF.to_string(),
            "rdfs" => RDFS.to_string(),
            "xsd" => XSD.to_string(),
            "v" => self.vocab.clone(),
            _ => {
                return self
                    .prefixes
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, i)| i.clone())
            }
        })
    }

    /// Resolves a name written in a query to an IRI. `quoted` names may hold CURIEs
    /// or absolute IRIs.
    pub fn resolve_text(&self, text: &str, quoted: bool) -> Result<String, String> {
        if !quoted || !text.contains(':') {
            return Ok(format!("{}{}", self.vocab, pct_encode(text)));
        }
        let (pre, local) = text.split_once(':').unwrap_or((text, ""));
        if let Some(base) = self.prefix_iri(pre) {
            return Ok(format!("{base}{local}"));
        }
        if is_absolute_iri(text) {
            return Ok(text.to_string());
        }
        Err(format!(
            "`{text}` is neither a declared CURIE nor an absolute IRI"
        ))
    }

    /// Renders an IRI as a Cypher name (design Decision 9).
    pub fn render(&self, iri: &str) -> String {
        if let Some(rest) = iri.strip_prefix(&self.vocab) {
            if !rest.is_empty() && !rest.contains(':') {
                return pct_decode(rest);
            }
        }
        let mut best: Option<(usize, String, String)> = None;
        let mut cands: Vec<(String, String)> = BUILTIN_PREFIXES
            .iter()
            .filter_map(|p| self.prefix_iri(p).map(|i| (p.to_string(), i)))
            .collect();
        cands.extend(self.prefixes.iter().cloned());
        for (name, base) in cands {
            if base.is_empty() || !iri.starts_with(&base) {
                continue;
            }
            let better = match &best {
                None => true,
                Some((l, n, _)) => base.len() > *l || (base.len() == *l && name < *n),
            };
            if better {
                best = Some((base.len(), name, base));
            }
        }
        match best {
            Some((l, name, _)) => format!("{name}:{}", &iri[l..]),
            None => iri.to_string(),
        }
    }

    /// Resolves an `@id` string: a declared CURIE, an absolute IRI or a skolem IRI.
    pub fn resolve_id(&self, s: &str) -> Result<Value, String> {
        let iri = if let Some((pre, local)) = s.split_once(':') {
            match self.prefix_iri(pre) {
                Some(base) => format!("{base}{local}"),
                None if is_absolute_iri(s) => s.to_string(),
                None => return Err(format!("invalid @id `{s}`")),
            }
        } else {
            return Err(format!("invalid @id `{s}`: not a CURIE or an IRI"));
        };
        Ok(Value::iri(iri).canonical())
    }

    /// The `@id` text that resolves back to `v` (element id of a node).
    pub fn element_id(v: &Value) -> String {
        v.lexical()
    }
}

impl Vocab {
    /// Reads the current `@vocab` and prefix table of the database
    /// (`(sys:db sys:vocab ?v)` and `(sys:db sys:prefix [name; iri])`).
    pub fn load(e: &mut dyn Executor) -> Result<Vocab> {
        let (v, prefixes) = read_settings(e)?;
        let mut out = Vocab::default();
        if let Some(v) = v {
            out.vocab = v;
        }
        out.prefixes = prefixes;
        Ok(out)
    }
}

/// The `@vocab` IRI and the `(name, iri)` prefix table of a database.
pub type Settings = (Option<String>, Vec<(String, String)>);

/// Reads `(sys:db sys:vocab ?v)` and the prefix layers `(sys:db sys:prefix [name; iri])`.
pub fn read_settings(e: &mut dyn Executor) -> Result<Settings> {
    let now = ViewSpec::NOW;
    let reader = TermReader::new(64);
    let id = |e: &mut dyn Executor, iri: &str| TermReader::encode(e, &Value::iri(iri));
    let Some(db) = id(e, vocab::SYS_DB)? else {
        return Ok((None, Vec::new()));
    };
    let mut vocab_iri = None;
    if let Some(p) = id(e, vocab::SYS_VOCAB)? {
        for t in read::triples(e, &now, Some(db), Some(p), None)? {
            if let Value::Iri(s) = reader.decode(e, t.o, false)? {
                vocab_iri = Some(s);
            }
        }
    }
    let mut prefixes = Vec::new();
    let sys = |n: &str| format!("{}{n}", vocab::SYS);
    if let (Some(pp), Some(pn), Some(pi)) = (
        id(e, &sys("prefix"))?,
        id(e, &sys("prefixName"))?,
        id(e, &sys("prefixIri"))?,
    ) {
        for t in read::triples(e, &now, Some(db), Some(pp), None)? {
            let names = read::triples(e, &now, Some(t.o), Some(pn), None)?;
            let iris = read::triples(e, &now, Some(t.o), Some(pi), None)?;
            if let (Some(n), Some(i)) = (names.first(), iris.first()) {
                let n = reader.decode(e, n.o, false)?;
                let i = reader.decode(e, i.o, false)?;
                if let (Value::Str(n), Value::Iri(i)) = (n, i) {
                    prefixes.push((n, i));
                }
            }
        }
    }
    Ok((vocab_iri, prefixes))
}
