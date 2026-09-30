## ADDED Requirements

### Requirement: Time-respecting evaluation
A path evaluation MAY be time-respecting, with an optional start instant `after` (epoch ms). A time-respecting evaluation SHALL carry a time τ along each walk, starting at `after`, or at −∞ when `after` is absent. A stored hop over a statement with valid interval `[v_from, v_to)` (NULL meaning unbounded) SHALL be taken only when `v_to` is NULL or `v_to > τ`, and SHALL set τ to `max(τ, v_from)`, a NULL `v_from` leaving τ unchanged. A virtual hop (`sys:subject`, `sys:object`, `sys:predicate` or an inverse) SHALL always be allowed and SHALL leave τ unchanged. The view's valid-time selection and a graph set SHALL still apply to every hop, independently. The arrival of a walk is its final τ.

The modes SHALL mean:
- `REACH`: each end reachable by a time-respecting walk exactly once; `hops` is the length of a shortest such walk; the arrival is the smallest arrival over all such walks within the hop bound.
- `TRAIL`: every time-respecting trail, each with its own arrival.
- `ANY_SHORTEST` and `ALL_SHORTEST`: the paths of minimal length among time-respecting walks to each end (one, the lexicographically smallest by hop key, or all), each with its own arrival.

Row order SHALL be the deterministic order of the mode. Without the option, evaluation SHALL be unchanged.

#### Scenario: A chain in time order is reachable
- **WHEN** `(A p B)` is valid `[1, 5)`, `(B p C)` is valid `[3, 9)`, and `p+` is evaluated time-respecting from `A` in `REACH` mode
- **THEN** the ends are `B` with arrival 1 and `C` with arrival 3

#### Scenario: Overlapping intervals chain at the same instant
- **WHEN** `(A p B)` is valid `[1, 5)` and `(B p C)` is valid `[0, 2)`
- **THEN** `C` is reachable with arrival 1; and when `(B p C)` is valid `[0, 1)` instead, `C` is not reachable

#### Scenario: A start instant cuts early facts
- **WHEN** `(A p B)` is valid `[1, 5)` and `p` is evaluated time-respecting from `A` with `after = 6`
- **THEN** no row is returned; with `after = 2` the end `B` has arrival 2

#### Scenario: Unbounded intervals
- **WHEN** `(A p B)` has no valid time and `(B p C)` is valid from 7, unbounded, and `p+` is evaluated time-respecting from `A`
- **THEN** `B` has no arrival bound (−∞) and `C` has arrival 7

#### Scenario: A longer walk can arrive earlier
- **WHEN** `(A p C)` is valid `[10, 20)`, `(A p B)` is valid `[1, 2)`, `(B p C)` is valid `[1, 3)`, and `p+` is evaluated time-respecting from `A` in `REACH` mode
- **THEN** `C` has hops 1 and arrival 1

#### Scenario: Virtual hops are structural
- **WHEN** `(alice worksAt acme)` with eid `e1` is valid `[1, 2)`, `(belief9 supportedBy e1)` is valid `[5, 9)`, and `supportedBy/sys:subject` is evaluated time-respecting from `belief9`
- **THEN** the end `alice` is returned with arrival 5, although `e1` is no longer valid at 5

#### Scenario: Shortest time-respecting paths
- **WHEN** `(A p B)` is valid `[5, 9)`, `(B p D)` is valid `[0, 3)`, `(A p C)` is valid `[1, 2)`, `(C p E)` is valid `[1, 4)`, `(E p D)` is valid `[2, 6)`, and `p+` is evaluated time-respecting from `A` in `ALL_SHORTEST` mode
- **THEN** the only row for `D` has 3 hops through `C` and `E`, with arrival 2

#### Scenario: Result equals a brute-force enumeration
- **WHEN** random small graphs with random intervals are evaluated time-respecting in `REACH` mode with a hop bound
- **THEN** the ends, their hop counts and their arrivals equal those of an enumeration of every time-respecting walk up to the bound

## MODIFIED Requirements

### Requirement: Path result values
Each result row SHALL carry the start, the end, the hop count and an arrival. The arrival SHALL be absent for an evaluation that is not time-respecting; for a time-respecting one it SHALL be the row's arrival instant in epoch ms, or absent when it is −∞ (no start instant and no traversed statement with a `v_from`). In `TRAIL`, `ANY_SHORTEST` and `ALL_SHORTEST` modes, each row SHALL also carry a path value. The path value is the ordered node sequence, from the start to the end (hops + 1 nodes), and the ordered hop sequence (hops entries). Each hop entry records:
- the eid of the statement traversed;
- its predicate;
- whether it was traversed forward (subject to object) or inverse.

Nodes SHALL be reported as the same values that triple-pattern queries return for them. This includes statement eids that appear as nodes.

#### Scenario: Path value contents
- **WHEN** the store holds `(a knows b)` with eid `e1` and `(c knows b)` with eid `e2`, and the path `knows/^knows` is evaluated from `a` in `TRAIL` mode
- **THEN** the row has start `a`, end `c` and hops 2, the nodes `[a, b, c]`, and the hops `[(e1, knows, forward), (e2, knows, inverse)]`

#### Scenario: Reachability rows carry no path
- **WHEN** a path is evaluated in `REACH` mode
- **THEN** each row carries start, end and hops, and no path value

#### Scenario: Arrival only for time-respecting evaluation
- **WHEN** a path is evaluated without the time-respecting option
- **THEN** no row carries an arrival
