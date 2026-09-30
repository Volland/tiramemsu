# retraction-cascade Specification

## Purpose
Defines how retracting a statement recursively retracts every live statement built on it (annotations in subject position and references in object position) in the same transaction, how cycles terminate, how the size of a cascade is bounded and previewed, and which retraction kind each retracted statement records.

## Requirements

### Requirement: Cascade over subject and object positions
When a statement `e` is retracted by any operation, every statement that is live at that point of the transaction and has `e` as its subject or as its object SHALL be retracted in the same transaction, and the rule SHALL apply recursively to each of them. All statements retracted by one cascade SHALL receive the same `t_ret`, equal to the current transaction. Only statement eids SHALL propagate a cascade; `IRI`, `NODE`, `BNODE`, `TX` and literal values never do.

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

### Requirement: Retraction kinds
Every retracted statement SHALL record a `ret_kind`: `explicit` (0) for the statement named by a retract or matched by retract-matching; `cascade` (1) for every other statement reached from an explicit retraction; `supersede` (2) for every statement in the cascade set of a superseded statement, root included; and `cardinality` (3) for every statement in the cascade set of a statement replaced by a cardinality-one assert, root included. The report SHALL list each retracted eid with the same kind.

#### Scenario: Explicit retraction with cascade
- **WHEN** `e1` with annotation `e2` is retracted explicitly
- **THEN** `e1` has kind `explicit` and `e2` has kind `cascade`

#### Scenario: Cardinality replacement with annotations
- **WHEN** `:age` is flagged `sys:cardinality sys:one`, `e1 = (alice :age 30)` has annotation `e2 = (e1 :source :form)`, and `(alice :age 31)` is asserted
- **THEN** `e1` and `e2` both have kind `cardinality`

### Requirement: Cascades terminate on cycles
The cascade SHALL track visited statements so that every statement in a reference cycle is retracted exactly once and the cascade terminates.

#### Scenario: Two statements referencing each other
- **WHEN** `e7` and `e8` each use the other's eid as subject or object, and `e7` is retracted
- **THEN** both are retracted exactly once in the same transaction
- **AND** the report lists each of them exactly once

#### Scenario: Diamond
- **WHEN** `e2 = (e1 :p x)`, `e3 = (e1 :q y)` and `e4 = (e2 :r e3)` are live and `e1` is retracted
- **THEN** `e2`, `e3` and `e4` are each retracted once, and `e4` appears once in the report

### Requirement: Cascade size limit
The number of statements newly retracted by one cascade, counting its root, SHALL NOT exceed the transaction's `max_cascade` option (default 10 000). A cascade that would exceed it SHALL fail the whole transaction with `CascadeLimitExceeded { root, limit }` and leave no trace. The limit SHALL apply separately to each root retraction (each retract, each match of retract-matching, each supersede and each cardinality replacement).

#### Scenario: Limit exceeded
- **WHEN** a statement has 5 annotations and is retracted with `max_cascade = 5`
- **THEN** the transaction fails with `CascadeLimitExceeded` naming that statement and the limit 5
- **AND** the `tx`, `triple`, `term` and `meta` tables are unchanged

#### Scenario: Exactly at the limit
- **WHEN** a statement has 4 annotations and is retracted with `max_cascade = 5`
- **THEN** the transaction commits and 5 statements are retracted

#### Scenario: Limit applies per root
- **WHEN** retract-matching matches 3 unrelated statements with 3 annotations each and `max_cascade = 4`
- **THEN** the transaction commits and 12 statements are retracted

### Requirement: Dry-run preview of a cascade
A dry-run transaction SHALL report the complete set of statements every retraction would retract, with their kinds, while the `triple` and `tx` tables stay unchanged. Only the id counters in `meta` MAY advance. A read-only preview SHALL also be available on any view without the writer: `View::dependents(e)` on the now view SHALL list, as a set, exactly the statements and memberships that a dry-run retraction of the live statement `e` reports, as the `statement-dependents` capability specifies.

#### Scenario: Preview a large cascade
- **WHEN** a statement with 3 annotations and 2 references is retracted in a dry run
- **THEN** the report lists all 6 statements with their kinds
- **AND** all 6 statements are still live afterwards and no `tx` row was added

#### Scenario: Read-only preview
- **WHEN** the same statement's dependents are read on the now view
- **THEN** the result holds the same 6 statements, the root first
- **AND** no id counter in `meta` advances

### Requirement: Retracted structures remain visible in the past
A cascade SHALL only set `t_ret` and `ret_kind`, so the as-of view at `t_ret − 1` SHALL still show every statement of the cascaded structure.

#### Scenario: What a belief relied on
- **WHEN** `e1`, its annotation `e2` and its reference `e7` are retracted in transaction 20
- **THEN** the as-of view at transaction 19 contains `e1`, `e2` and `e7` with their original content
