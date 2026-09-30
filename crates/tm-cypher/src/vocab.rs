//! Name to IRI mapping (`lat.md/data-model#Vocabulary Mapping`, design Decision 9):
//! resolution of labels, types and keys through `@vocab` and the prefix table, and
//! rendering of IRIs back to names.

use tm_core::vocab::{RDF, SYS, TM, V, XSD};
use tm_core::Value;

use crate::ast::Name;
use crate::error::{CResult, CypherError};

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

    /// Resolves a [`Name`], reporting a `Parse` error at its span.
    pub fn resolve(&self, n: &Name) -> CResult<String> {
        self.resolve_text(&n.text, n.escaped)
            .map_err(|m| CypherError::parse(n.span, m))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn n(t: &str, esc: bool) -> Name {
        Name {
            text: t.into(),
            escaped: esc,
            span: crate::Span::new(0, t.len()),
        }
    }

    fn with_schema() -> Vocab {
        Vocab {
            prefixes: vec![("schema".into(), "https://schema.org/".into())],
            ..Vocab::default()
        }
    }

    // vocabulary-mapping "Case is preserved" / "Relationship type and key"
    #[test]
    fn bare_names_use_the_vocab_verbatim() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve(&n("Person", false)).unwrap(),
            "urn:tiramemsu:v:Person"
        );
        assert_eq!(
            v.resolve(&n("person", false)).unwrap(),
            "urn:tiramemsu:v:person"
        );
        assert_eq!(
            v.resolve(&n("WORKS_AT", false)).unwrap(),
            "urn:tiramemsu:v:WORKS_AT"
        );
    }

    // "Backticked name with a space"
    #[test]
    fn backticked_space_is_percent_encoded() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve(&n("Top Customer", true)).unwrap(),
            "urn:tiramemsu:v:Top%20Customer"
        );
        assert_eq!(v.render("urn:tiramemsu:v:Top%20Customer"), "Top Customer");
    }

    // "User prefix" / "Built-in rdfs prefix" / "v prefix equals @vocab"
    #[test]
    fn curies_resolve_through_the_table() {
        let v = with_schema();
        assert_eq!(
            v.resolve(&n("schema:name", true)).unwrap(),
            "https://schema.org/name"
        );
        assert_eq!(
            v.resolve(&n("rdfs:label", true)).unwrap(),
            "http://www.w3.org/2000/01/rdf-schema#label"
        );
        assert_eq!(
            v.resolve(&n("v:Person", true)).unwrap(),
            v.resolve(&n("Person", false)).unwrap()
        );
        assert_eq!(
            v.resolve(&n("sys:unique", true)).unwrap(),
            "urn:tiramemsu:sys:unique"
        );
    }

    // "Full IRI label" / "Invalid IRI"
    #[test]
    fn full_iris_and_invalid_names() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve(&n("https://schema.org/Organization", true))
                .unwrap(),
            "https://schema.org/Organization"
        );
        assert!(v.resolve(&n("1bad:<>", true)).is_err());
    }

    // "Local name" / "CURIE and full IRI" / "Local name containing a colon is not shortened"
    #[test]
    fn rendering() {
        let v = with_schema();
        assert_eq!(v.render("urn:tiramemsu:v:Person"), "Person");
        assert_eq!(v.render("https://schema.org/Person"), "schema:Person");
        assert_eq!(v.render("http://example.org/X"), "http://example.org/X");
        assert_eq!(v.render("urn:tiramemsu:v:schema:name"), "v:schema:name");
        assert_eq!(v.render("urn:tiramemsu:sys:unique"), "sys:unique");
    }

    // longest prefix wins, ties by smallest name
    #[test]
    fn longest_prefix_and_tie_break() {
        let v = Vocab {
            prefixes: vec![
                ("b".into(), "https://x.org/a/".into()),
                ("a".into(), "https://x.org/a/".into()),
                ("short".into(), "https://x.org/".into()),
            ],
            ..Vocab::default()
        };
        assert_eq!(v.render("https://x.org/a/n"), "a:n");
        assert_eq!(v.render("https://x.org/z"), "short:z");
    }

    // "Round trip of rendered labels"
    #[test]
    fn names_round_trip() {
        let v = with_schema();
        for iri in [
            "urn:tiramemsu:v:Person",
            "urn:tiramemsu:v:Top%20Customer",
            "https://schema.org/Person",
            "http://example.org/X",
            "urn:tiramemsu:v:schema:name",
            "urn:tiramemsu:sys:supersedes",
        ] {
            let r = v.render(iri);
            let back = v.resolve_text(&r, true).unwrap();
            assert_eq!(back, iri, "rendered as {r}");
        }
    }

    #[test]
    fn ids() {
        let v = Vocab::default();
        assert_eq!(
            v.resolve_id("v:alice").unwrap(),
            Value::iri("urn:tiramemsu:v:alice")
        );
        assert_eq!(
            v.resolve_id("urn:tiramemsu:node:12").unwrap(),
            Value::Node(12)
        );
        assert_eq!(
            v.resolve_id("urn:tiramemsu:bnode:3").unwrap(),
            Value::BNode(3)
        );
        assert!(matches!(
            v.resolve_id("urn:tiramemsu:stmt:7").unwrap(),
            Value::Stmt(_)
        ));
        assert!(v.resolve_id("not an iri").is_err());
    }
}
