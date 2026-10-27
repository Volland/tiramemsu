//! Result formats of `tm query`: aligned tables, SPARQL 1.1 JSON / CSV / TSV
//! results, N-Triples for `CONSTRUCT`, the bridge's JSON for Cypher, and plans.

use std::io::Write;

use clap::ValueEnum;
use tiramemsu::rdf::{render, write_ntriples, write_term, RdfTerm};
use tiramemsu::*;

use crate::io;

/// Result formats of `tm query`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Aligned columns, terms in N-Triples syntax.
    Table,
    /// SPARQL 1.1 JSON results; the bridge's JSON for Cypher.
    Json,
    /// SPARQL 1.1 CSV results.
    Csv,
    /// SPARQL 1.1 TSV results.
    Tsv,
    /// N-Triples, for `CONSTRUCT`.
    Ntriples,
}

fn wrong(format: Format, what: &str) -> Error {
    Error::unsupported(format!(
        "--format {} for {what}",
        format
            .to_possible_value()
            .map_or("?".into(), |v| v.get_name().to_string())
    ))
}

/// Prints a SPARQL result.
pub fn sparql(out: &mut dyn Write, r: &SparqlResult, format: Format) -> Result<()> {
    match (r, format) {
        (SparqlResult::Graph(g), Format::Ntriples | Format::Table) => {
            out.write_all(write_ntriples(g).as_bytes()).map_err(io)
        }
        (SparqlResult::Graph(_), f) => Err(wrong(f, "a CONSTRUCT result (use ntriples)")),
        (SparqlResult::Update(rep), _) => writeln!(
            out,
            "tx {}: {} asserted, {} retracted",
            rep.t.0,
            rep.asserted.len(),
            rep.retracted.len()
        )
        .map_err(io),
        (_, Format::Json) => writeln!(out, "{}", r.write_sparql_json()?).map_err(io),
        (SparqlResult::Boolean(b), _) => writeln!(out, "{b}").map_err(io),
        (SparqlResult::Solutions(s), Format::Table) => {
            let rows: Vec<Vec<String>> = s
                .rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| c.as_ref().map_or(String::new(), |v| write_term(&render(v))))
                        .collect()
                })
                .collect();
            table(out, &s.vars, &rows)
        }
        (SparqlResult::Solutions(s), Format::Csv) => {
            writeln!(out, "{}", s.vars.join(",")).map_err(io)?;
            for r in &s.rows {
                let cells: Vec<String> = r
                    .iter()
                    .map(|c| c.as_ref().map_or(String::new(), csv_cell))
                    .collect();
                write!(out, "{}\r\n", cells.join(",")).map_err(io)?;
            }
            Ok(())
        }
        (SparqlResult::Solutions(s), Format::Tsv) => {
            let head: Vec<String> = s.vars.iter().map(|v| format!("?{v}")).collect();
            writeln!(out, "{}", head.join("\t")).map_err(io)?;
            for r in &s.rows {
                let cells: Vec<String> = r
                    .iter()
                    .map(|c| c.as_ref().map_or(String::new(), |v| write_term(&render(v))))
                    .collect();
                writeln!(out, "{}", cells.join("\t")).map_err(io)?;
            }
            Ok(())
        }
        (SparqlResult::Solutions(_), f) => Err(wrong(f, "a SELECT result")),
    }
}

/// A SPARQL 1.1 CSV cell: IRIs and literals by their plain text, quoted when needed.
fn csv_cell(v: &Value) -> String {
    let plain = match render(v) {
        RdfTerm::Iri(i) => i,
        RdfTerm::Blank(b) => format!("_:{b}"),
        RdfTerm::Literal { lex, .. } => lex,
        t @ RdfTerm::Triple(_) => write_term(&t),
    };
    if plain.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", plain.replace('"', "\"\""))
    } else {
        plain
    }
}

/// Prints a Cypher result.
pub fn cypher(out: &mut dyn Write, r: &CypherResult, format: Format) -> Result<()> {
    let cell = |v: &CypherValue| match v.to_json() {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    match format {
        Format::Json => writeln!(out, "{}", r.to_json()).map_err(io),
        Format::Table => {
            let rows: Vec<Vec<String>> = r
                .rows
                .iter()
                .map(|row| row.iter().map(cell).collect())
                .collect();
            table(out, &r.columns, &rows)
        }
        Format::Csv | Format::Tsv => {
            let sep = if format == Format::Csv { "," } else { "\t" };
            writeln!(out, "{}", r.columns.join(sep)).map_err(io)?;
            for row in &r.rows {
                let cells: Vec<String> = row
                    .iter()
                    .map(|v| {
                        let c = cell(v);
                        if format == Format::Csv && c.contains([',', '"', '\n', '\r']) {
                            format!("\"{}\"", c.replace('"', "\"\""))
                        } else {
                            c
                        }
                    })
                    .collect();
                writeln!(out, "{}", cells.join(sep)).map_err(io)?;
            }
            Ok(())
        }
        Format::Ntriples => Err(wrong(format, "a Cypher result")),
    }
}

/// Aligned columns with a header rule.
fn table(out: &mut dyn Write, head: &[String], rows: &[Vec<String>]) -> Result<()> {
    let mut width: Vec<usize> = head.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (w, c) in width.iter_mut().zip(r) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |cells: &[String]| {
        cells
            .iter()
            .zip(&width)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join(" | ")
            .trim_end()
            .to_string()
    };
    writeln!(out, "{}", line(head)).map_err(io)?;
    writeln!(
        out,
        "{}",
        width
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-+-")
    )
    .map_err(io)?;
    for r in rows {
        writeln!(out, "{}", line(r)).map_err(io)?;
    }
    writeln!(
        out,
        "({} row{})",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    )
    .map_err(io)
}

/// Prints a plan: each region's kind and routing note, its steps, and the SQL.
pub fn explain(out: &mut dyn Write, ex: &Explain) -> Result<()> {
    if ex.short_circuit {
        writeln!(out, "empty by construction (no statement can match)").map_err(io)?;
    }
    for r in &ex.regions {
        writeln!(out, "region {:?} ({:?})", r.kind, r.note).map_err(io)?;
        for step in &r.query_plan {
            writeln!(out, "  {step}").map_err(io)?;
        }
    }
    if let Some(sql) = &ex.sql {
        writeln!(out, "\nSQL:\n{sql}").map_err(io)?;
    }
    Ok(())
}
