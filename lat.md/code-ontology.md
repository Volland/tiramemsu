# Code Ontology

A small shared vocabulary for what the code is meant to do: six kinds of intent and eleven kinds of typed link between intent, requirements and code. Use these names instead of inventing synonyms, so a query means the same thing for everyone.

The machine-readable schema is `Types/code-types.md`, and `Templates/` holds a starting note for each type. `tg ontology` prints every type and edge type the project declares, including any the project added to `Types/` itself, with the properties each edge takes; `tg ontology check` reports edges that do not fit. Derive what the code can tell you, write what only people know, and let `tg check`, `tg ontology check` and `tg cypher` tell you when the two disagree.

## Two layers

Intent is written by people and lives in `lat.md/`. Realization is the code, derived by `tg`, plus the typed links you add from code to intent.

Do not generate the intent layer from the code. A document derived from the implementation agrees with it by construction, so it can never reveal drift.

## Intent types

Mark a file with `type:` in its frontmatter and every section below its title gets that label, so `MATCH (s:Decision)` works in `tg cypher`.

- `Decision` is a choice that was made, with its reason and the alternatives rejected. An ADR is a Decision.
- `Requirement` is something the system SHALL do. One section per requirement.
- `Scenario` is a checkable example of a requirement. Test-spec sections are Scenarios.
- `Constraint` is a rule nobody here chose: a regulation, an SLA, a platform limit.
- `Concept` is a domain term with one exact meaning.
- `Change` is a unit of work that moves intent and code together, with a lifecycle.

Suggested files: `decisions.md` with `type: Decision`, `requirements.md` with `type: Requirement`, `constraints.md`, `glossary.md` with `type: Concept`. A file may hold several types: `type: [Requirement, Scenario]`.

## Edge types

An edge says how two things relate. Write it where its source lives: a typed edge line in a doc section, or an `@tg:` comment above a code declaration. Both become the same named edge in the graph, with sign and properties.

In a doc, after the section's leading paragraph:

```markdown
## Short-lived access tokens

Access tokens expire after 15 minutes and refresh tokens rotate on every use.

- motivated_by:: [[constraints#Session limit]] {weight: primary}
- supersedes:: [[decisions#Long-lived sessions]] {since: "2.3", reason: "audit finding"}
```

In code, above the declaration the edge starts at:

```
// @tg: implements:: [[openspec:auth#Token expiry]] {since: "2.3", confidence: partial}
// @tg: verifies:: [[openspec:auth#Token expiry#Expired token]] {level: integration}
// @tg: -contradicts:: [[decisions#Stateless gateway]] {until: "2026-Q4", ticket: "PLAT-481"}
```

| Edge | From → to | Properties | Means |
| --- | --- | --- | --- |
| `implements` | code → Requirement, Decision | `since`, `confidence` (full, partial), `note` | code realizes it |
| `verifies` | code → Requirement, Scenario | `since`, `level` (unit, integration, e2e, manual) | a test proves it |
| `-contradicts` | code, Requirement, Decision → Decision, Requirement, Constraint | `until`, `ticket`, `reason` | knowingly in conflict: debt, named |
| `supersedes` | Decision → Decision | `since`, `reason` | replaces an older decision |
| `motivated_by` | Decision → Constraint, Concept, Decision | `since`, `weight` (primary, secondary), `note` | the reason behind a decision |
| `constrains` | Constraint → Requirement, Decision, code | `strength` (must, should), `source`, `note` | a rule binds it |
| `refines` | Requirement, Scenario → Requirement | `note` | a more specific version |
| `depends_on` | Requirement, code → Requirement | `strength` (hard, soft), `note` | a dependency of intent, not an import |
| `defines` | Concept → anything | `note` | where a term gets its meaning |
| `introduced_by`, `changed_by` | anything → Change | `date`, `note` | provenance |

When requirements live in OpenSpec, target them as `[[openspec:<capability>#<Requirement>]]` and `[[openspec:<capability>#<Requirement>#<Scenario>]]`; they appear as `Requirement` and `Scenario` nodes and `tg trace` reports coverage.

## Discovering and extending the ontology

The table above is the bundled vocabulary, not a limit: run `tg ontology` before writing, and use the project's own types where they fit.

The project may have added types (a `Service`, a `Risk`, an `Incident`) or edge types (`owned_by`, `mitigates`) to `Types/`, and a domain type that fits beats a generic one. When a relation you need has no name, add it to `Types/` as an edge type with `from`, `to` and its properties, then use it; never write a one-off synonym of an existing edge. `tg ontology check` catches the misspelling, the wrong direction and the undeclared property.

## Rules of use

Short rules that keep the graph worth reading.

- **The minus sign means disagreement** and nothing else. Red dashed lines must always mean something is wrong or knowingly wrong.
- **Put facts about the relationship on the relationship:** `since`, `until`, `ticket`, `confidence: partial`, `strength: must`. A sentence can explain; the property can be queried.
- **Type the file, name the link.** Untyped sections and plain links are what lat.md already gives you; the value of the graph starts with `type:` and `motivated_by::`.
- **A type earns its place** only if you would ask a question that depends on it. If you cannot name the query, drop the type.
- **Start small.** Five sections, three `implements`, one `verifies`, then `tg check`. If the result teaches you nothing, stop.

## Questions it answers

Run these with `tg cypher --code annotated "..."`. Replace the edge type to ask the sibling question; the edge's properties are columns (`r.since`, `r.ticket`).

```cypher
// Requirements nothing implements
MATCH (s:Requirement) OPTIONAL MATCH (c)-[r:implements]->(s) WITH s, count(r) AS n WHERE n = 0 RETURN s.section

// Built but unproven: implemented, with no test
MATCH (s:Requirement)<-[:implements]-(c) OPTIONAL MATCH (t)-[v:verifies]->(s) WITH s, count(v) AS n WHERE n = 0 RETURN DISTINCT s.section

// What we knowingly do against our own decisions
MATCH (c)-[r:contradicts]->(d) RETURN c.symbol, d.section, r.until, r.ticket

// Decisions and the constraints behind them, strongest reason first
MATCH (d:Decision)-[r:motivated_by]->(c:Constraint) RETURN d.title, c.title, r.weight ORDER BY r.weight

// Hard rules and everything they bind
MATCH (c:Constraint)-[r:constrains {strength: "must"}]->(x) RETURN c.title, x.title, x.symbol
```

Agents can call the same queries through the `tg_cypher` MCP tool and read the typed edges instead of guessing which function is the rule and which is a workaround.

## Who uses it

Each role touches a different part of the same graph.

- **Developers** add `@tg:` edges while they work and run `tg check` and `tg ontology check` before they finish.
- **Architects** own Decisions, Constraints and Concepts, and read `contradicts` and `supersedes`.
- **Product** owns Requirements and Scenarios, tags them by risk and status, and asks what is unbuilt or unproven.
- **Agents** run `tg ontology` and search first, read the typed edges around the code they change, and record what they changed as typed sections with typed edges.
