# layer-hops Specification

## Purpose
Defines the virtual `sys:subject`, `sys:object` and `sys:predicate` hops and their inverses. A path uses them to step from a statement to its parts and back, so it can cross statement layers such as annotations, provenance and beliefs about facts. They are computed from the statement row, never stored.

## Requirements

### Requirement: Forward virtual hops
In a path expression, the predicates `sys:subject`, `sys:object` and `sys:predicate` SHALL each step from a statement eid `e` to that statement's subject, object or predicate. The step happens only when `e` is a statement that is visible in the path's view. From any value that is not a visible statement, a forward virtual hop SHALL have no neighbours. These hops SHALL NOT need any stored triple with those predicates.

#### Scenario: Subject hop
- **WHEN** statement `e1 = (alice worksAt acme)` is live and the path `sys:subject` is evaluated from `e1` under `Now`
- **THEN** the only end is `alice` with 1 hop

#### Scenario: Object hop
- **WHEN** the path `sys:object` is evaluated from `e1` under `Now`
- **THEN** the only end is `acme` with 1 hop

#### Scenario: Predicate hop
- **WHEN** the path `sys:predicate` is evaluated from `e1` under `Now`
- **THEN** the only end is the IRI `worksAt`

#### Scenario: Object hop to a literal
- **WHEN** statement `e2 = (e1 confidence 0.8)` is live and the path `sys:object` is evaluated from `e2`
- **THEN** the only end is the literal `0.8`

#### Scenario: Virtual hop from a plain node
- **WHEN** the path `sys:subject` is evaluated from the node `alice`
- **THEN** no row is returned and no error is raised

#### Scenario: No stored sys triples exist
- **WHEN** the store holds no statement whose predicate is `sys:subject`, and the path `sys:subject` is evaluated from a live statement `e1`
- **THEN** the subject of `e1` is returned

### Requirement: Inverse virtual hops
The inverse hops `^sys:subject`, `^sys:object` and `^sys:predicate` SHALL each step from a value `x` to every statement eid `e`, visible in the path's view, whose subject, object or predicate is `x`.

#### Scenario: Inverse subject hop finds annotations
- **WHEN** `e1 = (alice worksAt acme)`, `e2 = (e1 confidence 0.8)` and `e3 = (e1 sys:confirmedBy tx42)` are live, and the path `^sys:subject` is evaluated from `e1`
- **THEN** the ends are exactly `e2` and `e3`

#### Scenario: Inverse object hop finds references
- **WHEN** `e7 = (belief9 supportedBy e1)` is live and the path `^sys:object` is evaluated from `e1`
- **THEN** the only end is `e7`

#### Scenario: Inverse subject hop from an entity
- **WHEN** `e1 = (alice worksAt acme)` and `e4 = (alice name "Alice")` are live, and the path `^sys:subject` is evaluated from `alice`
- **THEN** the ends are exactly `e1` and `e4`

### Requirement: Paths cross layers
Virtual hops SHALL combine with stored-predicate steps and with every path operator. A single path can then go from an entity to a statement, from a statement to its annotations or referencing statements, and back to entities, to any layer depth.

#### Scenario: From a belief through a statement to its entities
- **WHEN** `e1 = (alice worksAt acme)` and `e7 = (belief9 supportedBy e1)` are live, and the path `supportedBy/(sys:subject|sys:object)` is evaluated from `belief9` in `REACH` mode
- **THEN** the ends are exactly `alice` and `acme`, each with 2 hops

#### Scenario: From an entity to the beliefs that rely on its facts
- **WHEN** the store is as in the previous scenario and the path `^sys:subject/^supportedBy` is evaluated from `alice` in `REACH` mode
- **THEN** the only end is `belief9`

#### Scenario: Arbitrary layer depth
- **WHEN** `e1 = (alice worksAt acme)`, `e2 = (e1 confidence 0.8)`, `e5 = (e2 source "crawler")`, `e7 = (belief9 supportedBy e1)` and `e8 = (e7 method "llm-extraction")` are live, and the path `(^sys:subject)+` is evaluated from `e1` in `REACH` mode
- **THEN** the ends are exactly `e2` (1 hop) and `e5` (2 hops). `e7` and `e8` are not reached, because `e7` references `e1` as its object, not as its subject.

#### Scenario: Mixed recursive layer walk
- **WHEN** the same store is used and the path `(^sys:subject|^sys:object)+` is evaluated from `e1` in `REACH` mode
- **THEN** the ends are exactly `e2`, `e5`, `e7` and `e8`

### Requirement: Virtual hops respect the view
A virtual hop SHALL traverse statement `e` only when `e` is visible in the path's view, for both forward and inverse hops. For tx-time and valid-time selection, virtual hops SHALL apply the same view predicates as stored-statement hops.

#### Scenario: Retracted statement is not traversed under Now
- **WHEN** `e1 = (alice worksAt acme)` was retracted in tx 30, and the path `sys:subject` is evaluated from `e1` under `Now`
- **THEN** no row is returned

#### Scenario: Retracted statement is traversed as of before its retraction
- **WHEN** the same path is evaluated from `e1` under `AsOf(29)`
- **THEN** the only end is `alice`

#### Scenario: Inverse hop hides retracted annotations
- **WHEN** annotation `e2 = (e1 confidence 0.8)` was retracted and `e3 = (e1 sys:confirmedBy tx42)` is live, and the path `^sys:subject` is evaluated from `e1` under `Now`
- **THEN** the only end is `e3`

#### Scenario: Virtual hop under valid time
- **WHEN** statement `e1` is valid `[2020-01-01, 2022-01-01)`, and the path `sys:object` is evaluated from `e1` under `ValidAt(2023-01-01)`
- **THEN** no row is returned

### Requirement: Virtual hops in path values and trails
In a path value, a virtual hop SHALL record the traversed statement's eid, the virtual predicate (`sys:subject`, `sys:object` or `sys:predicate`), and the direction. Its relationship identity for trail uniqueness SHALL be the pair (statement eid, virtual predicate). That identity is distinct from the identity of the statement traversed as a stored edge, and from the other virtual predicates on the same statement.

#### Scenario: Virtual hop recorded in the path
- **WHEN** `e7 = (belief9 supportedBy e1)` and `e1 = (alice worksAt acme)` are live, and the path `supportedBy/sys:subject` is evaluated from `belief9` in `TRAIL` mode
- **THEN** the row has nodes `[belief9, e1, alice]` and hops `[(e7, supportedBy, forward), (e1, sys:subject, forward)]`

#### Scenario: Stored edge and virtual hop of the same statement on one trail
- **WHEN** `e1 = (alice worksAt acme)` is live, and the path `worksAt/^sys:object/sys:subject` is evaluated from `alice` in `TRAIL` mode
- **THEN** one row is returned, with nodes `[alice, acme, e1, alice]` and the three hops `(e1, worksAt, forward)`, `(e1, sys:object, inverse)` and `(e1, sys:subject, forward)`, because their relationship identities are distinct

#### Scenario: Same virtual hop is not reused on a trail
- **WHEN** `e1 = (alice worksAt acme)` is live and the path `(sys:subject|^sys:subject)` repeated 1 to 4 times is evaluated from `e1` in `TRAIL` mode
- **THEN** the only path is `e1 -> alice`: stepping back from `alice` to `e1` would reuse the identity (e1, sys:subject), so no row returns to `e1`

### Requirement: Virtual predicates are reserved
Because the virtual hop predicates are in the reserved `sys:` namespace, user data SHALL NOT be able to assert them. A path SHALL always evaluate them from statement rows, never from stored triples.

#### Scenario: Asserting a virtual hop predicate is rejected
- **WHEN** a transaction asserts `(x sys:subject y)`
- **THEN** it fails with a `ReservedNamespace` error, and the path `sys:subject` from `x` still returns nothing when `x` is not a statement
