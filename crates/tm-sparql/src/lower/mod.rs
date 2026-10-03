//! Lowering: spargebra's algebra → the `tm-ir` logical IR (design D5).
//!
//! Every lowered query carries the SPARQL semantic flags (`Homomorphism`,
//! `Unbound`, `SetOfTriples`). Lowering is a pure function of the parsed query and
//! the [`Env`]; nothing here touches the database.

// @lat: [[query#Front Ends#SPARQL]]

pub mod agg;
pub mod bgp;
pub mod expr;
pub mod path;
pub mod pattern;
pub mod vars;

use std::collections::HashMap;

use spargebra::term::TriplePattern as SpTriple;
use spargebra::Query;
use tm_core::{Result, Value};
use tm_ir::validate::scope;
use tm_ir::{Expr, GraphSel, IrQuery, Op, Var};

use crate::dataset::{GraphDataset, ViewScope};
use crate::env::Env;
use crate::error::{unsupported, DESCRIBE};
use crate::parse::AssocHints;
use vars::VarAlloc;

/// The result form of a lowered query: how to turn the executor's rows into the
/// answer the text asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryForm {
    /// `SELECT`: the rows are the solutions.
    Select,
    /// `ASK`: true when there is at least one row.
    Ask,
    /// `CONSTRUCT`: the template is instantiated once per row.
    Construct {
        /// The template triples (with reifier and annotation desugaring).
        template: Vec<SpTriple>,
    },
}

/// A lowered query: the IR to run and how to read its rows.
///
/// The IR carries the SPARQL semantic flags, so an executor can run it
/// unchanged. `Display` on [`IrQuery`] prints it as text, which is what the
/// golden tests of this crate compare.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryPlan {
    /// The result form.
    pub form: QueryForm,
    /// The IR query with the SPARQL semantic flags.
    pub query: IrQuery,
}

/// The state of one lowering run.
pub struct Lowerer<'a> {
    pub(crate) env: &'a Env,
    pub(crate) vars: VarAlloc,
    bnodes: HashMap<String, Var>,
    /// spargebra's random aggregate variable names → deterministic internal ones.
    pub(crate) renames: HashMap<String, Var>,
    /// Text hints for re-associating arithmetic chains.
    pub(crate) assoc: AssocHints,
    /// The `FROM` / `FROM NAMED` (or `USING`) graphs of the request.
    pub(crate) dataset: GraphDataset,
    /// The graph selection of the pattern being lowered: the dataset's default graph,
    /// replaced inside a `GRAPH` block.
    pub(crate) active: GraphSel,
    /// How many triple patterns took a `GRAPH ?g` selector so far.
    pub(crate) graph_var_uses: u32,
    /// The innermost `SERVICE <urn:tiramemsu:tm:timeRespecting…>` scope being
    /// lowered, if any.
    pub(crate) temporal: Option<TemporalScope>,
}

/// The state of a time-respecting `SERVICE` scope (`lat.md/query#Temporal Path
/// Syntax`): its start instant, how many paths it made time-respecting, and the
/// `tm:arrival` patterns `(end, variable)` waiting for their path.
#[derive(Clone, Debug, Default)]
pub(crate) struct TemporalScope {
    pub(crate) after: Option<tm_ir::TermOrVar>,
    pub(crate) paths: u32,
    pub(crate) arrivals: Vec<(tm_ir::TermOrVar, Var)>,
}

impl<'a> Lowerer<'a> {
    /// A lowerer for `env`.
    pub fn new(env: &'a Env) -> Lowerer<'a> {
        Lowerer {
            env,
            vars: VarAlloc::new(),
            bnodes: HashMap::new(),
            renames: HashMap::new(),
            assoc: AssocHints::default(),
            dataset: GraphDataset::default(),
            active: GraphSel::Any,
            graph_var_uses: 0,
            temporal: None,
        }
    }

    /// Sets the graph dataset of the request; the default graph becomes the active
    /// selection.
    pub fn set_dataset(&mut self, dataset: GraphDataset) {
        self.active = dataset.default_selector();
        self.dataset = dataset;
    }

    /// The variable standing for the blank node label `label`.
    pub(crate) fn bnode_var(&mut self, label: &str) -> Var {
        if let Some(v) = self.bnodes.get(label) {
            return v.clone();
        }
        let v = self.vars.fresh("b");
        self.bnodes.insert(label.to_string(), v.clone());
        v
    }

    /// The IR variable of a SPARQL variable name (aggregate outputs are renamed).
    pub(crate) fn var(&self, name: &str) -> Var {
        self.renames
            .get(name)
            .cloned()
            .unwrap_or_else(|| Var::new(name))
    }

    /// The `NOW()` constant: fixed per query at the wall-clock start.
    pub(crate) fn now(&self) -> Value {
        Value::DateTime {
            ms: self.env.now_ms,
            tz: Some(0),
        }
    }
}

/// Lowers a parsed query (no text hints: arithmetic chains stay as parsed).
pub fn lower_query(q: &Query, env: &Env) -> Result<QueryPlan> {
    lower_query_with(q, env, AssocHints::default())
}

/// Lowers a parsed query with the hints computed from its text.
pub fn lower_query_with(q: &Query, env: &Env, assoc: AssocHints) -> Result<QueryPlan> {
    let mut l = Lowerer::new(env);
    l.assoc = assoc;
    l.set_dataset(GraphDataset::from_dataset(q.dataset())?);
    let root_scope = ViewScope::from_dataset(q.dataset())?;
    match q {
        Query::Describe { .. } => Err(unsupported(DESCRIBE)),
        Query::Select { pattern, .. } => {
            let op = l.pattern(pattern, root_scope)?;
            Ok(QueryPlan {
                form: QueryForm::Select,
                query: IrQuery::sparql(op),
            })
        }
        Query::Ask { pattern, .. } => {
            let op = l.pattern(pattern, root_scope)?;
            let flag = Var::new(format!("{}ask", vars::INTERNAL_PREFIX));
            let op = op
                .extend(flag.name(), Expr::Const(Value::Bool(true)))
                .order_limit(Vec::new(), None, Some(1));
            let op = Op::Project(tm_ir::Project {
                input: Box::new(op),
                vars: vec![flag],
                distinct: false,
            });
            Ok(QueryPlan {
                form: QueryForm::Ask,
                query: IrQuery::sparql(op),
            })
        }
        Query::Construct {
            template, pattern, ..
        } => {
            let op = l.pattern(pattern, root_scope)?;
            let visible = vars::visible(&scope(&op).vars);
            let op = Op::Project(tm_ir::Project {
                input: Box::new(op),
                vars: visible,
                distinct: false,
            });
            Ok(QueryPlan {
                form: QueryForm::Construct {
                    template: template.clone(),
                },
                query: IrQuery::sparql(op),
            })
        }
    }
}
