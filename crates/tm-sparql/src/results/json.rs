//! SPARQL 1.1 Query Results JSON.

use std::fmt::Write;

use tm_core::{Eid, Value};

use super::term::{render, RdfTerm};
use super::Solutions;

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn term(t: &RdfTerm, out: &mut String) {
    match t {
        RdfTerm::Iri(s) => {
            out.push_str("{\"type\":\"uri\",\"value\":");
            escape(s, out);
            out.push('}');
        }
        RdfTerm::Blank(b) => {
            out.push_str("{\"type\":\"bnode\",\"value\":");
            escape(b, out);
            out.push('}');
        }
        RdfTerm::Literal {
            lex,
            datatype,
            lang,
        } => {
            out.push_str("{\"type\":\"literal\",\"value\":");
            escape(lex, out);
            if let Some(l) = lang {
                out.push_str(",\"xml:lang\":");
                escape(l, out);
            } else if let Some(d) = datatype {
                out.push_str(",\"datatype\":");
                escape(d, out);
            }
            out.push('}');
        }
        RdfTerm::Triple(_) => {
            // SELECT results never carry triple terms; keep the document valid
            out.push_str("{\"type\":\"uri\",\"value\":\"urn:tiramemsu:triple-term\"}");
        }
    }
}

/// The JSON document of a `SELECT`: `head.vars` in projection order, one object
/// per row in `results.bindings` omitting unbound variables. Row order is kept.
///
/// When the solutions carry provenance, the non-standard top-level member
/// `"provenance"` sits between `head` and `results`: one array of statement IRIs
/// per binding, in the same order. It precedes `results` because streaming
/// parsers (such as `sparesults`) stop at the end of the bindings and reject
/// anything after them. Without provenance only the standard members are written.
pub fn write_select(sol: &Solutions) -> String {
    let mut out = String::from("{\"head\":{\"vars\":[");
    for (i, v) in sol.vars.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        escape(v, &mut out);
    }
    out.push_str("]},");
    if let Some(prov) = &sol.provenance {
        write_provenance(prov, &mut out);
        out.push(',');
    }
    out.push_str("\"results\":{\"bindings\":[");
    for (ri, row) in sol.rows.iter().enumerate() {
        if ri > 0 {
            out.push(',');
        }
        out.push('{');
        let mut first = true;
        for (v, cell) in sol.vars.iter().zip(row) {
            let Some(cell) = cell else { continue };
            if !first {
                out.push(',');
            }
            first = false;
            escape(v, &mut out);
            out.push(':');
            term(&render(cell), &mut out);
        }
        out.push('}');
    }
    out.push_str("]}}");
    out
}

/// `"provenance":[[<statement IRI>, …], …]`.
fn write_provenance(prov: &[Vec<Eid>], out: &mut String) {
    out.push_str("\"provenance\":[");
    for (ri, eids) in prov.iter().enumerate() {
        if ri > 0 {
            out.push(',');
        }
        out.push('[');
        for (i, e) in eids.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if let RdfTerm::Iri(iri) = render(&Value::Stmt(*e)) {
                escape(&iri, out);
            }
        }
        out.push(']');
    }
    out.push(']');
}

/// The JSON document of an `ASK`.
pub fn write_ask(answer: bool) -> String {
    format!("{{\"head\":{{}},\"boolean\":{answer}}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};

    fn sample() -> Solutions {
        Solutions {
            vars: vec!["p".into(), "age".into(), "note".into()],
            rows: vec![
                vec![
                    Some(Value::iri("urn:tiramemsu:v:bob")),
                    None,
                    Some(Value::str("a\"b\n")),
                ],
                vec![
                    Some(Value::Stmt(tm_core::Eid::new(12))),
                    Some(Value::Int(41)),
                    Some(Value::LangStr {
                        lex: "hi".into(),
                        lang: "en".into(),
                    }),
                ],
            ],
            provenance: None,
        }
    }

    // sparql-query "SPARQL JSON results": SELECT JSON document
    #[test]
    fn select_document_shape() {
        let sol = Solutions {
            vars: vec!["p".into(), "age".into()],
            rows: vec![vec![Some(Value::iri("urn:tiramemsu:v:bob")), None]],
            provenance: None,
        };
        let want = r#"{"head":{"vars":["p","age"]},"results":{"bindings":[{"p":{"type":"uri","value":"urn:tiramemsu:v:bob"}}]}}"#;
        assert_eq!(write_select(&sol), want);
    }

    // sparql-query "SPARQL JSON results": Typed literal JSON
    #[test]
    fn typed_literal_json() {
        let sol = Solutions {
            vars: vec!["a".into()],
            rows: vec![vec![Some(Value::Int(41))]],
            provenance: None,
        };
        assert!(write_select(&sol).contains(
            r#"{"type":"literal","value":"41","datatype":"http://www.w3.org/2001/XMLSchema#integer"}"#
        ));
    }

    // sparql-query "SPARQL JSON results": ASK JSON document
    #[test]
    fn ask_document() {
        assert_eq!(write_ask(true), r#"{"head":{},"boolean":true}"#);
        assert_eq!(write_ask(false), r#"{"head":{},"boolean":false}"#);
    }

    // the output parses back with sparesults
    #[test]
    fn output_parses_back() {
        let doc = write_select(&sample());
        let parser = QueryResultsParser::from_format(QueryResultsFormat::Json);
        let ReaderQueryResultsParserOutput::Solutions(sols) =
            parser.for_reader(doc.as_bytes()).unwrap()
        else {
            panic!("not solutions")
        };
        assert_eq!(sols.variables().len(), 3);
        let rows: Vec<_> = sols.map(|r| r.unwrap()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("age"), None);
        assert_eq!(rows[0].get("note").unwrap().to_string(), "\"a\\\"b\\n\"");
        assert_eq!(rows[1].get("note").unwrap().to_string(), "\"hi\"@en");
        let ask_doc = write_ask(true);
        let ask = QueryResultsParser::from_format(QueryResultsFormat::Json)
            .for_reader(ask_doc.as_bytes())
            .unwrap();
        assert!(matches!(ask, ReaderQueryResultsParserOutput::Boolean(true)));
    }

    // query-provenance "Provenance in SPARQL JSON": JSON member
    // @lat: [[tests#Query Provenance#Provenance JSON Member]]
    #[test]
    fn provenance_member() {
        let mut sol = sample();
        let plain = write_select(&sol);
        assert!(!plain.contains("provenance"));
        sol.provenance = Some(vec![vec![Eid::new(5), Eid::new(7)], Vec::new()]);
        let doc = write_select(&sol);
        let prov = r#""provenance":[["urn:tiramemsu:stmt:5","urn:tiramemsu:stmt:7"],[]],"#;
        assert_eq!(doc.replace(prov, ""), plain, "{doc}");
        assert!(doc.contains(&format!(r#"]}},{prov}"results":"#)), "{doc}");
        // still a SPARQL 1.1 JSON results document
        let ReaderQueryResultsParserOutput::Solutions(sols) =
            QueryResultsParser::from_format(QueryResultsFormat::Json)
                .for_reader(doc.as_bytes())
                .unwrap()
        else {
            panic!("not solutions")
        };
        let rows: Vec<_> = sols.map(|r| r.unwrap()).collect();
        assert_eq!(rows.len(), 2);
    }
}
