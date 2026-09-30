//! Tiramemsu logical query IR: the front-end-neutral algebra that SPARQL, Cypher
//! and the programmatic API lower to (`lat.md/query#Logical IR`).
//!
//! An [`IrQuery`] is an operator tree ([`Op`]) plus the query's [`Semantics`].
//! Every triple and path pattern carries its own [`View`].
//!
//! ```
//! use tm_ir::builder::IrBuilder;
//! use tm_ir::{Expr, Op, View};
//!
//! // "what did alice work at as of tx 150, and where does she work now?"
//! let b = IrBuilder::sparql();
//! let before = b.at(View::as_of_tx(150)).triple("v:alice", "v:worksAt", "?before");
//! let after = b.triple("v:alice", "v:worksAt", "?after");
//! let q = b.query(
//!     Op::join(vec![before, after]).filter(Expr::ne(Expr::var("before"), Expr::var("after"))),
//! );
//! assert!(q.to_string().contains("asOf/tx:150"));
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod agg;
pub mod builder;
pub mod display;
pub mod expr;
pub mod op;
pub mod params;
pub mod path;
pub mod semantics;
pub mod term;
pub mod validate;
pub mod var;
pub mod view;
pub mod vocab;

pub use agg::{Agg, AggFunc, Key};
pub use expr::{ArithOp, CmpOp, Expr, Func, Lookup, LookupMode};
pub use op::{
    Aggregate, Extend, Filter, GraphSel, IrQuery, Join, LeftJoin, Op, OrderLimit, PathPattern,
    Project, RowNumber, TriplePattern, Union, Unnest, Values,
};
pub use params::{params, Params};
pub use path::{PathExpr, PathMode};
pub use semantics::{GraphSet, MatchMode, Missing, Semantics};
pub use term::TermOrVar;
pub use var::{Var, VarSet};
pub use view::{TimeRef, TxSel, ValidSel, View};
