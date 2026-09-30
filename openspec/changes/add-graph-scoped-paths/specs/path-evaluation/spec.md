## ADDED Requirements

### Requirement: Graph-scoped evaluation
A path evaluation MAY carry a graph set G (a list of graph ObjectIds). With a graph set, a hop SHALL be taken only when the statement it traverses is a member of at least one graph of G, the membership `(e sys:inGraph g)` being visible in the same view as the hop. The traversed statement SHALL be:
- for a stored hop (a predicate atom, its inverse, or the wildcard), the statement stepped over;
- for a virtual hop `sys:subject`, `sys:object` or `sys:predicate`, or its inverse, the statement whose part is stepped to or from.

Zero-hop rows SHALL NOT depend on G. An empty G, graph ids that have no membership, and a database in which `sys:inGraph` was never written SHALL all give only the zero-hop rows. Without a graph set, evaluation SHALL be unchanged.

#### Scenario: Path confined to a graph
- **WHEN** `(a knows b)` and `(b knows c)` are members of `g1`, `(c knows d)` is a member of `g2` only, and `knows+` is evaluated from `a` in `REACH` mode with G = `[g1]`
- **THEN** the ends are `b` and `c`, and with G = `[g1, g2]` they are `b`, `c` and `d`

#### Scenario: A statement in two graphs of the set is one hop
- **WHEN** `(a knows b)` is a member of both `g1` and `g2`, and `knows` is evaluated from `a` in `TRAIL` mode with G = `[g1, g2]`
- **THEN** exactly one row is returned

#### Scenario: Membership read in the hop's view
- **WHEN** `(a knows b)` and `(b knows c)` are members of `g1` from tx 10, the membership of `(b knows c)` is removed in tx 20, and `knows+` is evaluated from `a` with G = `[g1]`
- **THEN** under `AsOf(15)` the ends are `b` and `c`, and under `Now` the only end is `b`

#### Scenario: Virtual hops need the stepped statement in the graph
- **WHEN** `(alice worksAt acme)` has eid `e1` and is a member of `g1`, `(belief9 supportedBy e1)` is a member of `g1`, and `supportedBy/sys:subject` is evaluated from `belief9` with G = `[g1]`
- **THEN** the end is `alice`; and when `e1` is a member of `g2` only, no row is returned

#### Scenario: No membership predicate
- **WHEN** the database never wrote `sys:inGraph`, and `knows*` is evaluated from `a` with G = `[g1]`
- **THEN** the only row is `a` with 0 hops
