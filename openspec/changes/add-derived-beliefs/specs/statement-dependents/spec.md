## MODIFIED Requirements

### Requirement: Dependents of a statement

`read::dependents(exec, spec, root)` and `View::dependents(root)` SHALL return `root` followed by every statement visible in the view that is reachable from `root` by repeatedly taking a statement whose subject or object is an already reached statement, or a statement `D` for which a visible support link `(D sys:derivedFrom x)` to an already reached statement `x` exists. The walk SHALL be breadth-first with a visited set, so every statement SHALL appear exactly once and a reference cycle SHALL terminate. The order SHALL be that of the cascade: `root` first, then the statements in breadth-first order, where the statements found from one reached statement are appended in ascending eid order. When `root` is not visible in the view the result SHALL be empty. Only statement eids SHALL propagate the walk; nodes, literals and transactions never do. The read SHALL NOT write, SHALL NOT take the writer lock outside a speculation, and SHALL run in one read snapshot.

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

#### Scenario: Derived beliefs are dependents
- **WHEN** `D` was derived from `e1` with link `L = (D sys:derivedFrom e1)` and `dependents(e1)` is read on the now view
- **THEN** the result contains `e1`, `L` and `D`

### Requirement: Dependents equal the cascade

Under the now view, `dependents(e)` of a live statement `e` SHALL equal, as a set, the statements a retraction of `e` would retract: the retracted statements and retracted memberships of a dry-run `retract(e)`. It SHALL also equal the set of ends of the path `(^sys:subject|^sys:object|^sys:derivedFrom)*` from `e` in reachability mode.

#### Scenario: Random layered graphs
- **WHEN** random layered graphs with random retractions and random derivations are generated, and for a live statement `e` the dependents, a dry-run retraction of `e` and the path ends from `e` are computed on the now view
- **THEN** the three sets are equal
