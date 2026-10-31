## Context

`add-elpi-engine` runs text programs. Datalog stores its programs as `dl:source` nodes and checks enforced programs in a pre-commit hook (`datalog_enforce::commit_check`) that reads the transaction's own state through `ReadStore`.

## Decisions

### Resolution

`tm_elpi_store::resolve(store, spec, root)` returns the program's parts in dependency order. Each part has its node, its source eid and its text with the `accumulate <iri>.` lines removed. A text program given to `View::elpi` resolves its includes the same way. The engine concatenates the parts.

### The commit hook

`tm_elpi_store::commit_check(tx, dry_run, budget)` reads the `elpi:enforced` marks of the transaction's state. With no mark the cost is one dictionary lookup. A program whose mark or source (or an included source) was added in this transaction gets the full check. Every other program gets the delta check, with seeds equal to the rows added and retracted in this transaction. For a violation that cites no statement added in this transaction, the hook asks the view as of the previous transaction for the same severity and name. The violation is dropped when one of those answers cites the same eids.

### Budget

`ConstraintBudget::max_seed_rows` becomes the callback row cap. The deadline becomes fuel: `timeout_ms × 4000` resolution steps, at least 10⁵. lean-elpi runs a few million steps a second, so a looping check stops close to the deadline. If either runs out, the transaction fails with `ConstraintBudgetExceeded`.

## Risks / Trade-offs

- [A delta form that misses a case lets a violation through] → the full check runs on enable and on every text change, and `View::elpi_check` finds missed cases. A property test comparing the delta form with the full one is not written yet.
