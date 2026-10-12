//! Vector index commands and `CALL tiramemsu.vector.setEmbedding` (`lat.md/query#Vector
//! Recall#Writes From Query Languages`). A vector index is an embedding model:
//! creating one defines the model, dropping one retracts its definition, and
//! `SHOW` lists the models defined in the view. Search stays exact, so no derived
//! structure is built.

use std::collections::BTreeMap;

use tm_core::vector::{EmbeddingModel, Metric, VectorEncoding};
use tm_core::{Error, Value};
use tm_ir::{Op, TermOrVar, TriplePattern};

use super::{Exec, Row};
use crate::ast::{Expr, Name};
use crate::error::{CResult, CypherError};
use crate::parse::schema::{IndexOptions, SchemaCommand};
use crate::program::CypherParams;
use crate::value::{CypherResult, CypherValue, Val};
use crate::vocab::VocabExt;

/// The write procedure's name (compared lower-case).
pub const SET_EMBEDDING: &str = "tiramemsu.vector.setembedding";

/// The columns of `SHOW VECTOR INDEXES`.
pub const SHOW_COLUMNS: [&str; 5] = ["name", "model", "dimensions", "metric", "encoding"];

fn opt_err(msg: impl Into<String>) -> CypherError {
    CypherError::eval(format!("CREATE VECTOR INDEX OPTIONS: {}", msg.into()))
}

fn int(v: &CypherValue, key: &str) -> CResult<u32> {
    match v {
        CypherValue::Integer(n) => {
            u32::try_from(*n).map_err(|_| opt_err(format!("{key} is out of range")))
        }
        _ => Err(opt_err(format!("{key} is an integer"))),
    }
}

fn text<'v>(v: &'v CypherValue, key: &str) -> CResult<&'v str> {
    match v {
        CypherValue::String(s) => Ok(s),
        _ => Err(opt_err(format!("{key} is a string"))),
    }
}

fn metric(s: &str) -> CResult<Metric> {
    match s.to_ascii_lowercase().as_str() {
        "euclidean" => Ok(Metric::L2),
        other => Metric::from_name(other)
            .ok_or_else(|| opt_err(format!("unknown metric `{s}` (cosine, dot, l2, euclidean)"))),
    }
}

/// The model definition an `OPTIONS` map describes: tiramemsu keys `dimensions`,
/// `metric` and `encoding`, or Neo4j's `indexConfig` with `vector.dimensions` and
/// `vector.similarity_function`.
pub fn definition(opts: &BTreeMap<String, CypherValue>) -> CResult<EmbeddingModel> {
    let mut dims = None;
    let mut def = EmbeddingModel::new(0);
    for (k, v) in opts {
        match k.as_str() {
            "dimensions" => dims = Some(int(v, k)?),
            "metric" => def.metric = metric(text(v, k)?)?,
            "encoding" => {
                def.encoding = VectorEncoding::from_name(&text(v, k)?.to_ascii_lowercase())
                    .ok_or_else(|| opt_err("encoding is `f32` or `f16`"))?
            }
            "indexConfig" => {
                let CypherValue::Map(cfg) = v else {
                    return Err(opt_err("indexConfig is a map"));
                };
                for (ck, cv) in cfg {
                    match ck.as_str() {
                        "vector.dimensions" => dims = Some(int(cv, ck)?),
                        "vector.similarity_function" => def.metric = metric(text(cv, ck)?)?,
                        other => return Err(opt_err(format!("unknown indexConfig key `{other}`"))),
                    }
                }
            }
            other => return Err(opt_err(format!("unknown key `{other}`"))),
        }
    }
    def.dimensions = dims.ok_or_else(|| opt_err("dimensions is required"))?;
    Ok(def)
}

impl Exec<'_> {
    fn model_iri(&self, name: &Name) -> CResult<Value> {
        Ok(Value::Iri(self.vocab.resolve(name)?))
    }

    /// Runs a vector index command.
    // @lat: [[query#Vector Recall#Writes From Query Languages]]
    pub(crate) fn run_schema(
        &mut self,
        cmd: &SchemaCommand,
        params: &CypherParams,
    ) -> CResult<CypherResult> {
        let mut out = CypherResult {
            columns: Vec::new(),
            rows: Vec::new(),
            report: None,
            path_completeness: None,
        };
        match cmd {
            SchemaCommand::Create {
                name,
                if_not_exists,
                options,
            } => {
                let opts = match options {
                    IndexOptions::Map(m) => m.clone(),
                    IndexOptions::Param(p) => match params.get(p) {
                        Some(CypherValue::Map(m)) => m.clone(),
                        _ => return Err(opt_err(format!("${p} is not a map"))),
                    },
                };
                let def = definition(&opts)?;
                let model = self.model_iri(name)?;
                let shown = name.text.clone();
                let if_not_exists = *if_not_exists;
                self.tx_do(|tx| {
                    if !if_not_exists && tx.embedding_model(&model)?.is_some() {
                        return Err(Error::invalid_query(format!(
                            "vector index `{shown}` already exists (use IF NOT EXISTS)"
                        )));
                    }
                    tx.define_embedding_model(&model, &def).map(|_| ())
                })?;
            }
            SchemaCommand::Drop { name, if_exists } => {
                let model = self.model_iri(name)?;
                let shown = name.text.clone();
                let if_exists = *if_exists;
                self.tx_do(|tx| {
                    if !tx.drop_embedding_model(&model)? && !if_exists {
                        return Err(Error::invalid_query(format!(
                            "there is no vector index `{shown}` (use IF EXISTS)"
                        )));
                    }
                    Ok(())
                })?;
            }
            SchemaCommand::Show => {
                out.columns = SHOW_COLUMNS.iter().map(|c| c.to_string()).collect();
                out.rows = self.show_indexes()?;
            }
        }
        Ok(out)
    }

    /// The models defined in the view, one row each, by model IRI.
    fn show_indexes(&mut self) -> CResult<Vec<Vec<CypherValue>>> {
        let view = self.view();
        let pat = |p: &str, o: &str| {
            Op::Triple(TriplePattern::new(
                TermOrVar::var("m"),
                TermOrVar::iri(p),
                TermOrVar::var(o),
                view,
            ))
        };
        let op = Op::join(vec![
            pat(tm_core::vocab::SYS_DIMENSIONS, "d"),
            pat(tm_core::vocab::SYS_METRIC, "k"),
            pat(tm_core::vocab::SYS_VECTOR_ENCODING, "e"),
        ]);
        let res = self.run_op(op)?;
        let col = |n: &str| res.col(n);
        let mut rows = Vec::new();
        for r in &res.rows {
            let get = |n: &str| col(n).and_then(|i| r[i].clone());
            let (
                Some(Value::Iri(m)),
                Some(Value::Int(d)),
                Some(Value::Str(k)),
                Some(Value::Str(e)),
            ) = (get("m"), get("d"), get("k"), get("e"))
            else {
                continue;
            };
            rows.push(vec![
                CypherValue::String(self.vocab.render(&m)),
                CypherValue::String(m),
                CypherValue::Integer(d),
                CypherValue::String(k),
                CypherValue::String(e),
            ]);
        }
        rows.sort_by(|a, b| format!("{:?}", a[1]).cmp(&format!("{:?}", b[1])));
        rows.dedup();
        Ok(rows)
    }

    /// `CALL tiramemsu.vector.setEmbedding(statement, model, vector) YIELD stored`,
    /// once per input row: stores the embedding of a relationship or statement
    /// node; `stored` is true when the vector is new. A `null` argument skips the row.
    pub(crate) fn exec_set_embedding(
        &mut self,
        rows: Vec<Row>,
        args: &[Expr],
        yields: &[(Name, Option<Name>)],
        standalone: bool,
    ) -> CResult<(Vec<Row>, Vec<String>)> {
        let err = |m: String| CypherError::eval(format!("tiramemsu.vector.setEmbedding: {m}"));
        if args.len() != 3 {
            return Err(err(format!("takes 3 arguments, got {}", args.len())));
        }
        let out_col = match yields {
            [] => standalone.then(|| "stored".to_string()),
            [(n, a)] if n.text == "stored" => Some(a.as_ref().unwrap_or(n).text.clone()),
            _ => return Err(err("the only yield column is `stored`".into())),
        };
        let mut out = Vec::new();
        for r in rows {
            let stmt = match self.eval(&args[0], &r)? {
                Val::Rel(e) => e,
                Val::Node(Value::Stmt(e)) => e,
                Val::Null => continue,
                other => {
                    return Err(err(format!(
                        "the statement is a {}, not a relationship",
                        other.type_name()
                    )))
                }
            };
            let model = match self.eval(&args[1], &r)? {
                Val::Str(s) => Value::Iri(self.vocab.resolve_text(&s, true).map_err(err)?),
                Val::Null => continue,
                other => return Err(err(format!("the model is a {}", other.type_name()))),
            };
            let vector: Vec<f32> = match self.eval(&args[2], &r)? {
                Val::List(xs) => xs
                    .iter()
                    .map(|x| match x {
                        Val::Int(i) => Ok(*i as f32),
                        Val::Float(f) => Ok(*f as f32),
                        _ => Err(err("the vector is a list of numbers".into())),
                    })
                    .collect::<CResult<_>>()?,
                Val::Null => continue,
                other => return Err(err(format!("the vector is a {}", other.type_name()))),
            };
            let stored = self.tx_do(|tx| tx.set_embedding(stmt, &model, &vector))?;
            let mut nr = r.clone();
            if let Some(c) = &out_col {
                nr.insert(c.clone(), Val::Bool(stored));
            }
            out.push(nr);
        }
        Ok((out, out_col.into_iter().collect()))
    }
}
