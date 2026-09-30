## ADDED Requirements

### Requirement: Dependents of a statement

`read::dependents(exec, spec, root)` and `View::dependents(root)` SHALL return `root` followed by every statement visible in the view that is reachable from `root` by repeatedly taking a statement whose subject or object is an already reached statement. The walk SHALL be breadth-first with a visited set, so every statement SHALL appear exactly once and a reference cycle SHALL terminate. The order SHALL be that of the cascade: `root` first, then the statements in breadth-first order, where the statements found from one reached statement are appended in ascending eid order. When `root` is not visible in the view the result SHALL be empty. Only statement eids SHALL propagate the walk; nodes, literals and transactions never do. The read SHALL NOT write, SHALL NOT take the writer lock outside a speculation, and SHALL run in one read snapshot.

#### Scenario: Layers and references
- **WHEN** `e1 = (alice worksAt acme)`, `e2 = (e1 confidence 0.8)`, `e7 = (belief9 supportedBy e1)` and `e8 = (e7 method "llm-extraction")` are live, and `dependents(e1)` is read on the now view
- **THEN** the result is `[e1, e2, e7, e8]`

#### Scenario: Plain nodes do not propagate
- **WHEN** `(alice name "Alice")` is also live
- **THEN** it is not in `dependents(e1)`

#### Scenario: Reference cycle terminates
- **WHEN** `e7` and `e8` each use the other's eid as subject or object, and `dependents(e7)` is read
- **THEN** the result is `[e7, e8]`, each once

#### Scenario: Not visible
- **WHEN** `dependents` is read for a retracted eid on the now view, or for an eid that never existed
- **THEN** the result is empty

#### Scenario: Retracted layers are not walked
- **WHEN** `e2 = (e1 note "a")` was retracted and `e3 = (e2 note "b")` is live, and `dependents(e1)` is read on the now view
- **THEN** neither `e2` nor `e3` is in the result

### Requirement: Dependents follow the view

The walk SHALL use the time predicates of the view, produced by the single view-predicate function, for the root and for every expansion. Under an as-of view the result SHALL be the statements that depended on the root at that transaction, including statements retracted since. Under the history view it SHALL be every statement that ever depended on the root, including statements retracted before it. Under a valid-at filter only statements valid at the instant SHALL be walked.

#### Scenario: What depended on it back then
- **WHEN** `e1` with layers `e2` and `e7` exists as of transaction 3, and `e1` is retracted with its cascade in transaction 4
- **THEN** `dependents(e1)` on the now view is empty, and on the as-of view at transaction 3 it is `[e1, e2, e7]`

#### Scenario: Everything that ever depended
- **WHEN** a layer `e2` on `e1` was retracted in transaction 3 and a layer `e3` on `e1` was asserted in transaction 4
- **THEN** `dependents(e1)` on the history view contains `e2` and `e3`, and on the now view only `e3`

#### Scenario: Valid-at filter
- **WHEN** `e2 = (e1 role "lead")` is valid only in 2024 and `dependents(e1)` is read with `validAt` in 2025
- **THEN** `e2` is not in the result

### Requirement: Dependents equal the cascade

Under the now view, `dependents(e)` of a live statement `e` SHALL equal, as a set, the statements a retraction of `e` would retract: the retracted statements and retracted memberships of a dry-run `retract(e)`. It SHALL also equal the set of ends of the path `(^sys:subject|^sys:object)*` from `e` in reachability mode.

#### Scenario: Random layered graphs
- **WHEN** random layered graphs with random retractions are generated, and for a live statement `e` the dependents, a dry-run retraction of `e` and the path ends from `e` are computed on the now view
- **THEN** the three sets are equal

### Requirement: Dependents are an unbounded read

`dependents` SHALL NOT truncate its result and SHALL NOT fail because of its size. The cascade limit `max_cascade` SHALL apply to retractions only.

#### Scenario: Larger than the cascade limit
- **WHEN** a statement has 20 layers
- **THEN** `dependents` returns all 21 statements, while a retraction with `max_cascade = 10` fails with `CascadeLimitExceeded`

### Requirement: Dependents in the JSON bridge

The JSON bridge SHALL offer the read `dependents` with `eid` (a number or `{"stmt": n}`) and `view`, returning the list of eids as numbers in the order above.

#### Scenario: Bridge read
- **WHEN** a statement with one layer is asserted and `call("dependents", {"eid": e})` is called
- **THEN** the result is `[e, layer]`
