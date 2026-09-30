//! The native path engine (`lat.md/query#Physical Planning#Path Engine`): a
//! breadth-first search over the product of a DFA and the graph, one batched
//! neighbour fetch per layer, behind three surfaces that share one code path: the
//! planner's `PathPattern` regions and raw SQL through the `tm_path` table
//! function, and the facade's `View::path`.

pub mod ast;
pub mod automaton;
pub mod engine;
pub mod fetch;
pub mod resolve;
pub mod row;
pub mod search;
pub mod syntax;
pub mod view;
pub mod vtab;

pub use engine::{PathEngine, PathOptions, PathRequest};
pub use row::{Hop, Path, PathRow};
pub use vtab::PathOperator;
