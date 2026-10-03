# tm-ir

The logical query IR of tiramemsu: one small algebra that SPARQL, Cypher and the Rust API all lower to.

`tm-ir` defines the operator tree ([`Op`]), the query wrapper ([`IrQuery`]), the per-pattern time [`View`], the three semantic flags ([`Semantics`]), structural validation and a canonical text printer. It has no SQL, no I/O and depends only on `tm-core` for values. It is a data-structure crate; nothing runs here.

```text
  SPARQL (tm-sparql)   openCypher (tm-cypher)   Rust API (tiramemsu)
            \                   |                   /
             +----------->  tm-ir  <---------------+     <- you are here
                     (Op tree + View per pattern + Semantics)
                              |
                          tm-exec  (plan, SQL, paths, decode)
                              |
                     tm-core / tm-rusqlite  (one SQLite file)
```

Most applications should use the [`tiramemsu`](https://docs.rs/tiramemsu) facade crate, which re-exports what you need and executes queries for you. Depend on `tm-ir` directly when you are writing a new front end (a query language that lowers to the IR), a rewriter or analyser over IR trees, or tests that assert on the IR a front end produced.

## Usage

Build a query with [`builder::IrBuilder`], validate it, and print it. The example asks what Alice worked at as of transaction 150 and what she works at now; each triple pattern carries its own view, so both live in one query.

```rust
use tm_ir::builder::IrBuilder;
use tm_ir::validate::{output_vars, validate};
use tm_ir::{Expr, Op, View};

let b = IrBuilder::sparql();
let before = b.at(View::as_of_tx(150)).triple("v:alice", "v:worksAt", "?before");
let after = b.triple("v:alice", "v:worksAt", "?after");
let q = b.query(
    Op::join(vec![before, after])
        .filter(Expr::ne(Expr::var("before"), Expr::var("after")))
        .project(&["before", "after"]),
);

validate(&q)?; // structural checks, no database needed
let cols: Vec<String> = output_vars(&q.root).iter().map(|v| v.to_string()).collect();
assert_eq!(cols.len(), 2);

let text = q.to_string(); // canonical S-expression; stable enough for snapshot tests
assert!(text.contains("asOf/tx:150"));

// A malformed tree is rejected before any SQL exists: `Extend` may not rebind a bound variable.
let bad = b.query(b.triple("?a", "v:p", "?b").extend("a", Expr::val(tm_core::Value::Int(1))));
assert!(validate(&bad).is_err());
# Ok::<(), tm_core::Error>(())
```

## Tour

- [`Op`] and its node structs ([`TriplePattern`], [`PathPattern`], [`TextPattern`] (text recall), [`Join`], [`LeftJoin`], [`Filter`], [`Union`], [`Extend`], [`Aggregate`], [`Project`], [`OrderLimit`], [`Unnest`], [`RowNumber`], [`Values`]): the algebra.
- [`IrQuery`]: a root [`Op`] plus [`Semantics`].
- [`View`], [`TxSel`], [`ValidSel`], [`TimeRef`]: the time selection of one pattern.
- [`Semantics`], [`MatchMode`], [`Missing`], [`GraphSet`]: the dialect flags.
- [`Expr`], [`Func`], [`Lookup`]: scalar expressions, including `Exists` and per-row property lookups.
- [`PathExpr`], [`PathMode`]: property-path expressions and enumeration modes.
- [`TermOrVar`], [`Var`], [`VarSet`], [`Params`]: pattern positions, variables and `$name` parameters.
- [`builder`]: [`builder::IrBuilder`], with CURIE expansion for `v:`, `sys:`, `tm:`, `xsd:`, `rdf:`.
- [`validate`]: [`validate::validate`], [`validate::scope`], [`validate::output_vars`].
- [`display`]: the canonical S-expression printer and a small path-text parser.
- [`vocab`]: the reserved `sys:` and `tm:` IRIs (virtual predicates, `sys:inGraph`).

## Design notes

- **Operators.** Leaves are view-scoped triple patterns and path patterns; everything else is relational: join, left join, filter, union, extend, aggregate, project, order/limit, plus `Values`, `Unnest` and `RowNumber` for Cypher. A `Join` with no inputs is the unit relation.
- **A view per pattern.** Every [`TriplePattern`], [`PathPattern`] and [`TextPattern`] holds its own [`View`] (`Now`, `AsOf`, `History` for transaction time; `Unfiltered` or `At(ms)` for valid time). The default is `{Now, Unfiltered}`. Front ends fill views in explicitly, using [`View::overlay`] for query-level and per-pattern clauses, so a query can compare two points in time. Time predicates are written by exactly one function in `tm-exec`, never here.
- **Semantic flags.** Instead of two algebras there is one algebra and three flags: [`MatchMode`] (`Homomorphism` or `RelIsomorphism`), [`Missing`] (`Unbound` or `Null3VL`) and [`GraphSet`] (`SetOfTriples` or `BagOfEids`). [`Semantics::sparql`] and [`Semantics::cypher`] are the two presets; `IrBuilder::sparql()` and `IrBuilder::cypher()` use them. Other combinations are legal.
- **Validation.** [`validate::validate`] rejects structurally invalid trees with `InvalidQuery`: rebinding an already bound variable in `Extend`, `Unnest` or `RowNumber`, aggregate outputs that collide with group variables, `Values` rows of the wrong width, negative skip or limit, and a virtual-predicate pattern that binds an eid. It does not check that constants exist in a database; that happens at plan time in `tm-exec`.
- **Text form.** `IrQuery` and `Op` implement `Display` as an indented S-expression that starts with a `; flags` line. It is canonical and is what snapshot tests compare. It is for humans and tests, not a serialisation format to parse back.
- **Parameters.** `$name` positions are bound by the executor from a [`Params`] map; a missing one is an error naming it.

## Status

Version 0.1. The IR is used by the in-repo front ends but the public API may still change between 0.x releases; enums such as [`Op`] and [`Func`] may gain variants. Minimum supported Rust version: 1.88.

## Links

- Repository: <https://github.com/Volland/tiramemsu>
- Logical IR design: <https://github.com/Volland/tiramemsu/blob/main/lat.md/query.md>
- Architecture: <https://github.com/Volland/tiramemsu/blob/main/lat.md/architecture.md>
- Executor crate: <https://docs.rs/tm-exec>

## License

MIT OR Apache-2.0
