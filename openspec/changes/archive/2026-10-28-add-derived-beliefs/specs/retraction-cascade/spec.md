## MODIFIED Requirements

### Requirement: Cascade over subject and object positions
When a statement `e` is retracted by any operation, every statement that is live at that point of the transaction and has `e` as its subject or as its object SHALL be retracted in the same transaction, and the rule SHALL apply recursively to each of them. In addition, every live statement `D` for which a live support link `(D sys:derivedFrom e)` exists SHALL be retracted in the same transaction, recursively, so a derived belief falls with any of its supports. All statements retracted by one cascade SHALL receive the same `t_ret`, equal to the current transaction. Only statement eids SHALL propagate a cascade; `IRI`, `NODE`, `BNODE`, `TX` and literal values never do.

#### Scenario: Annotation and reference are retracted
- **WHEN** `e1 = (alice worksAt acme)`, `e2 = (e1 :confidence 0.8)` and `e7 = (:belief9 :supportedBy e1)` are live and `e1` is retracted in transaction 12
- **THEN** `e1`, `e2` and `e7` all have `t_ret = 12`

#### Scenario: Recursive layers
- **WHEN** additionally `e8 = (e7 :method "llm-extraction")` is live and `e1` is retracted
- **THEN** `e8` is retracted in the same transaction, because its subject `e7` was retracted

#### Scenario: Plain nodes do not cascade
- **WHEN** `(alice worksAt acme)` is retracted while `(alice :name "Alice")` and `(acme :name "Acme")` are live
- **THEN** both name statements stay live

#### Scenario: Transaction metadata survives
- **WHEN** `e1` has a confirmation `(e1 sys:confirmedBy tx5)` and transaction 5 has metadata `(tx5 sys:author :agent7)`, and `e1` is retracted
- **THEN** the confirmation statement is retracted and the author statement stays live

#### Scenario: Already retracted statements are not walked
- **WHEN** `e2 = (e1 :note "a")` was retracted earlier, `e3 = (e2 :note "b")` is live, and `e1` is retracted
- **THEN** `e2` keeps its original `t_ret` and `ret_kind`
- **AND** `e3` stays live, because the cascade only follows live statements

#### Scenario: Derived belief follows its support
- **WHEN** `D = (alice :prefers tea)` was derived from `e1` and `e1` is retracted in transaction 12
- **THEN** the link `(D sys:derivedFrom e1)` and `D` both have `t_ret = 12`
