%% lat:begin %%
# Before starting work

- Run `lat search` to find sections relevant to your task. Read them to understand the design intent before writing code.
- Run `lat expand` on user prompts to expand any `[[refs]]` — this resolves section names to file locations and provides context.

# Post-task checklist (REQUIRED — do not skip)

After EVERY task, before responding to the user:

- [ ] Update `lat.md/` if you added or changed any functionality, architecture, tests, or behavior
- [ ] Run `lat check` — all wiki links and code refs must pass
- [ ] Do not skip these steps. Do not consider your task done until both are complete.

---

# What is lat.md?

This project uses [lat.md](https://www.npmjs.com/package/lat.md) to maintain a structured knowledge graph of its architecture, design decisions, and test specs in the `lat.md/` directory. It is a set of cross-linked markdown files that describe **what** this project does and **why** — the domain concepts, key design decisions, business logic, and test specifications. Use it to ground your work in the actual architecture rather than guessing.

# Commands

```bash
lat locate "Section Name"      # find a section by name (exact, fuzzy)
lat refs "file#Section"        # find what references a section
lat search "natural language"  # semantic search across all sections
lat expand "user prompt text"  # expand [[refs]] to resolved locations
lat check                      # validate all links and code refs
```

Run `lat --help` when in doubt about available commands or options.

If `lat search` fails because no API key is configured, explain to the user that semantic search requires a key provided via `LAT_LLM_KEY` (direct value), `LAT_LLM_KEY_FILE` (path to key file), or `LAT_LLM_KEY_HELPER` (command that prints the key). Supported key prefixes: `sk-...` (OpenAI) or `vck_...` (Vercel). If the user doesn't want to set it up, use `lat locate` for direct lookups instead.

# Syntax primer

- **Section ids**: `lat.md/path/to/file#Heading#SubHeading` — full form uses project-root-relative path (e.g. `lat.md/tests/search#RAG Replay Tests`). Short form uses bare file name when unique (e.g. `search#RAG Replay Tests`, `cli#search#Indexing`).
- **Wiki links**: `[[target]]` or `[[target|alias]]` — cross-references between sections. Can also reference source code: `[[src/foo.ts#myFunction]]`.
- **Source code links**: Wiki links in `lat.md/` files can reference functions, classes, constants, and methods in TypeScript/JavaScript/Python/Rust/Go/C files. Use the full path: `[[src/config.ts#getConfigDir]]`, `[[src/server.ts#App#listen]]` (class method), `[[lib/utils.py#parse_args]]`, `[[src/lib.rs#Greeter#greet]]` (Rust impl method), `[[src/app.go#Greeter#Greet]]` (Go method), `[[src/app.h#Greeter]]` (C struct). `lat check` validates these exist.
- **Code refs**: `// @lat: [[section-id]]` (JS/TS/Rust/Go/C) or `# @lat: [[section-id]]` (Python) — ties source code to concepts

# Test specs

Key tests can be described as sections in `lat.md/` files (e.g. `tests.md`). Add frontmatter to require that every leaf section is referenced by a `// @lat:` or `# @lat:` comment in test code:

```markdown
---
lat:
  require-code-mention: true
---
# Tests

Authentication and authorization test specifications.

## User login

Verify credential validation and error handling for the login endpoint.

### Rejects expired tokens
Tokens past their expiry timestamp are rejected with 401, even if otherwise valid.

### Handles missing password
Login request without a password field returns 400 with a descriptive error.
```

Every section MUST have a description — at least one sentence explaining what the test verifies and why. Empty sections with just a heading are not acceptable. (This is a specific case of the general leading paragraph rule below.)

Each test in code should reference its spec with exactly one comment placed next to the relevant test — not at the top of the file:

```python
# @lat: [[tests#User login#Rejects expired tokens]]
def test_rejects_expired_tokens():
    ...

# @lat: [[tests#User login#Handles missing password]]
def test_handles_missing_password():
    ...
```

Do not duplicate refs. One `@lat:` comment per spec section, placed at the test that covers it. `lat check` will flag any spec section not covered by a code reference, and any code reference pointing to a nonexistent section.

# Section structure

Every section in `lat.md/` **must** have a leading paragraph — at least one sentence immediately after the heading, before any child headings or other block content. The first paragraph must be ≤250 characters (excluding `[[wiki link]]` content). This paragraph serves as the section's overview and is used in search results, command output, and RAG context — keeping it concise guarantees the section's essence is always captured.

```markdown
# Good Section

Brief overview of what this section documents and why it matters.

More detail can go in subsequent paragraphs, code blocks, or lists.

## Child heading

Details about this child topic.
```

```markdown
# Bad Section

## Child heading

Details about this child topic.
```

The second example is invalid because `Bad Section` has no leading paragraph. `lat check` validates this rule and reports errors for missing or overly long leading paragraphs.
%% lat:end %%

%% tg:begin %%
# Before starting work

- Run `tg search` to find sections relevant to your task. Read them with `tg section` to understand the design intent before writing code.
- Run `tg expand` on user prompts to expand any `[[refs]]` — this resolves section names to file locations and provides context.
- Run `tg ontology` once per session to learn this project's types and edge types. Use those names; never invent a synonym for a relation the ontology already has.

# Post-task checklist (REQUIRED — do not skip)

After EVERY task, before responding to the user:

- [ ] Update `lat.md/` if you added or changed any functionality, architecture, decisions, tests, or behavior — as typed sections with typed edges (see below), not only prose
- [ ] Run `tg check` — all wiki links, code refs and annotations must pass
- [ ] Run `tg ontology check` — every typed edge you wrote must use a declared edge type with allowed endpoints and properties
- [ ] Do not skip these steps. Do not consider your task done until all three pass.

---

# What is lat.md?

This project keeps its intent — architecture, decisions, constraints, requirements and test specs — in the `lat.md/` directory as a **typed graph**: cross-linked markdown sections whose files have types (`Decision`, `Requirement`, `Constraint`, ...) and whose links have names (`motivated_by`, `constrains`, `implements`, ...) and properties (`since`, `ticket`, `confidence`). The `tg` CLI (Typed Graph) reads, checks and queries it, and is compatible with the lat.md format. Use it to ground your work in the actual architecture rather than guessing, and to answer questions with a query instead of a grep.

# Commands

```bash
tg search "natural language"  # find sections (lexical; hybrid with embeddings)
tg section "file#Section"     # read a section with its links and code references
tg locate "Section Name"      # find a section by name (exact, fuzzy)
tg refs "file#Section"        # what references a section
tg expand "user prompt text"  # expand [[refs]] to resolved locations
tg ontology                   # the project's types, edge types, endpoints and edge properties (from Types/)
tg ontology check             # typed edges in docs and code that the ontology does not allow
tg cypher "MATCH (d:Decision)-[r:motivated_by]->(c) RETURN d.title, c.title, r.since"   # query docs and code
tg edges --type implements    # typed edges written from code
tg check                      # validate links, code refs, indexes and annotations
tg migrate --dry-run          # from plain lat.md: type files, turn links into typed edges (see the tg-migrate skill)
```

Run `tg --help` when in doubt about available commands or options.

# The ontology

`Types/` holds the project's ontology as schema notes; `tg ontology` prints it, `lat.md/code-ontology.md` explains how to use it. The bundled code ontology has six types — **Decision**, **Requirement**, **Scenario**, **Constraint**, **Concept**, **Change** — and edge types such as `implements`, `verifies`, `-contradicts`, `supersedes`, `motivated_by`, `constrains`, `refines`, `depends_on`, `defines`, `changed_by`. A project may add its own types and edges to `Types/`: check `tg ontology` rather than assuming the list above, and use a domain type wherever it fits better than a generic one.

- **Type every file of intent.** Put `type:` in the frontmatter: `type: Decision` for `decisions.md`, `type: [Requirement, Scenario]` for a mixed file. Every section below the title gets that label in queries.
- **Name every relation.** When a section relates to another section, a requirement or code, write the relation as a typed edge line, not only as prose.
- **Put facts about a relation on the edge**: `{since: "2.3"}`, `{ticket: "PLAT-481", until: "2026-Q4"}`, `{confidence: partial}`, `{strength: must}`. `tg ontology` lists the properties each edge type takes.
- **The minus sign means disagreement** and nothing else: `-contradicts::` marks something knowingly in conflict.
- **A relation the ontology lacks** is a schema change: add the edge type to `Types/` (with `from`, `to` and properties) instead of inventing a one-off name, then use it.

# Writing typed docs

A section keeps its leading paragraph first; typed edge lines follow as a list. Each line is an ordinary `[[wiki link]]` for `tg check`, and a named edge with properties in the graph:

```markdown
---
type: Decision
---
# Decisions

Choices we made, with their reasons and the alternatives we rejected.

## Short-lived access tokens

Access tokens expire after 15 minutes and refresh tokens rotate on every use.

- motivated_by:: [[constraints#Session limit]] {weight: primary, since: "2.3"}
- supersedes:: [[decisions#Long-lived sessions]] {reason: "audit finding"}
- -contradicts:: [[decisions#Stateless gateway]] {ticket: "PLAT-481", until: "2026-Q4"}

The refresh endpoint keeps a small revocation list, which is why the gateway is no longer fully stateless.
```

Edges can point at sections (`[[file#Section]]`), at OpenSpec requirements and scenarios (`[[openspec:<capability>#<Requirement>]]`) and at code (`[[src/auth/token.ts#refresh]]`).

# Syntax primer

- **Section ids**: `lat.md/path/to/file#Heading#SubHeading` — full form uses the project-root-relative path. Short form uses the bare file name when unique (e.g. `search#RAG Replay Tests`).
- **Wiki links**: `[[target]]` or `[[target|alias]]` — cross-references between sections. Can also reference source code: `[[src/foo.ts#myFunction]]`, `[[src/server.ts#App#listen]]` (class method).
- **Typed doc edges**: `- type:: [[target]] {key: value}` on its own line in a section — a named edge from that section; `-type::` makes it negative.
- **Typed code edges**: `// @tg: implements:: [[decisions#Short-lived access tokens]] {since: "2.3"}` — a named edge from the next declaration; a bare `// @tg: [[x]]` or `// @lat: [[x]]` is a plain link.
- **OpenSpec requirements**: if the project has `openspec/`, point code at the requirement it realizes with `// @tg: implements:: [[openspec:<capability>#<Requirement>]]` and tests at the scenario they prove with `// @tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]` directly above the `it(`. List the capabilities a lat.md file explains as `openspec: [capability]` in its frontmatter. `tg trace --gaps` shows what is unimplemented or unverified.

# Questions to ask the graph

Use `tg cypher` (or the `tg_cypher` MCP tool) before reading files one by one; add `--code annotated` to include code:

```cypher
// Why does this decision exist?
MATCH (d:Decision {title: "Short-lived access tokens"})-[r]->(x) RETURN type(r), x.title, r
// Requirements nothing implements
MATCH (q:Requirement) OPTIONAL MATCH (c)-[i:implements]->(q) WITH q, count(i) AS n WHERE n = 0 RETURN q.title
// Known debt, with its ticket and deadline
MATCH (a)-[r:contradicts]->(b) RETURN a.title, b.title, r.ticket, r.until
```

# Test specs

Key tests can be described as sections in `lat.md/` files (e.g. `tests.md`, typed `Scenario`). Add frontmatter to require that every leaf section is referenced by an `@lat:` or `@tg:` comment in test code:

```markdown
---
type: Scenario
lat:
  require-code-mention: true
---
# Tests

## User login

Verify credential validation and error handling for the login endpoint.

### Rejects expired tokens
Tokens past their expiry timestamp are rejected with 401, even if otherwise valid.

- refines:: [[requirements#Token expiry]]
```

Each test references its spec with exactly one comment next to the test:

```python
# @lat: [[tests#User login#Rejects expired tokens]]
def test_rejects_expired_tokens():
    ...
```

`tg check` flags any spec section not covered by a code reference, and any reference to a nonexistent section.

# Section structure

Every section in `lat.md/` **must** have a leading paragraph — at least one sentence right after the heading, before any child heading or edge list. The first paragraph must be ≤250 characters (excluding `[[wiki link]]` content). It is the section's overview and is used in search results and command output.
%% tg:end %%
