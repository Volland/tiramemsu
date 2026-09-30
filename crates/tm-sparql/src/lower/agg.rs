//! `Group` → `Aggregate` (design D5).

use spargebra::algebra::{AggregateExpression, AggregateFunction, GraphPattern};
use spargebra::term::Variable;
use tm_core::Result;
use tm_ir::{Agg, AggFunc, Op, Var};

use super::Lowerer;
use crate::dataset::ViewScope;
use crate::error::{unsupported, CUSTOM_AGGREGATE};

impl Lowerer<'_> {
    /// Lowers a `Group` node: grouping variables plus the aggregates. An
    /// aggregate query without `GROUP BY` has an empty grouping list and forms one
    /// group, even over zero solutions.
    pub fn group(
        &mut self,
        inner: &GraphPattern,
        variables: &[Variable],
        aggregates: &[(Variable, AggregateExpression)],
        sc: ViewScope,
    ) -> Result<Op> {
        let input = self.pattern(inner, sc)?;
        let group: Vec<Var> = variables.iter().map(|v| self.var(v.as_str())).collect();
        let mut aggs = Vec::new();
        for (var, agg) in aggregates {
            // spargebra names aggregate outputs randomly; keep the IR deterministic
            let internal = self.vars.fresh("a");
            self.renames.insert(var.as_str().to_string(), internal);
            aggs.push(self.aggregate(var, agg, sc)?);
        }
        Ok(Op::Aggregate(tm_ir::Aggregate {
            input: Box::new(input),
            group,
            aggs,
        }))
    }

    fn aggregate(
        &mut self,
        var: &Variable,
        agg: &AggregateExpression,
        sc: ViewScope,
    ) -> Result<Agg> {
        match agg {
            AggregateExpression::CountSolutions { distinct } => {
                if *distinct {
                    return Err(unsupported("COUNT(DISTINCT *)"));
                }
                Ok(Agg::count_star(self.var(var.as_str()).name()))
            }
            AggregateExpression::FunctionCall {
                name,
                expr,
                distinct,
            } => {
                let func = match name {
                    AggregateFunction::Count => AggFunc::Count,
                    AggregateFunction::Sum => AggFunc::Sum,
                    AggregateFunction::Avg => AggFunc::Avg,
                    AggregateFunction::Min => AggFunc::Min,
                    AggregateFunction::Max => AggFunc::Max,
                    AggregateFunction::Sample => AggFunc::Sample,
                    AggregateFunction::GroupConcat { separator } => AggFunc::GroupConcat {
                        sep: separator.clone().unwrap_or_else(|| " ".to_string()),
                    },
                    AggregateFunction::Custom(_) => return Err(unsupported(CUSTOM_AGGREGATE)),
                };
                let arg = self.expr(expr, sc)?;
                let a = Agg::new(self.var(var.as_str()).name(), func, arg);
                Ok(if *distinct { a.distinct() } else { a })
            }
        }
    }
}
