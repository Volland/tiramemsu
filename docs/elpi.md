# ELPI in Tiramemsu: a guide

ELPI is Tiramemsu's third reasoning language, after N3 and Datalog. It is a dialect of λProlog with higher-order clauses, hypothetical reasoning (`=>`, `pi`) and constraint handling rules, run by [lean-elpi](https://github.com/Volland/lean-elpi), an implementation in Lean 4. Use it for validators that need search, rules about rules, or questions of the form "what would follow if…". This guide covers the calls, the terms, time, constraints, stored programs and materialization. The [long read](../site/articles/elpi-reasoning.html) explains the design, and the decisions are in `lat.md/elpi.md` (E1 to E13).

ELPI is the opt-in facade feature `elpi` (crates `tm-elpi` and `tm-elpi-store`). It links the native library `libleanelpi`, which carries the Lean runtime and exists for macOS and Linux on arm64 and x86-64. Point `LEANELPI_DIST` at the directory that holds it. No build turns ELPI on by default, and the Node, Python and wasm packages leave it out. The CLI and the MCP server have the feature as well.

**Status.** Built and tested. It needs lean-elpi 0.4, which adds host callbacks and structured answers to its C interface and is not on crates.io yet, so for now the workspace builds against a lean-elpi checkout next to the repository.

## Five minutes

Store two statements, then ask a question in ELPI:

```rust
let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("manager"), v("bob"), Valid::ALWAYS)?;
    tx.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
    Ok(())
})?;
let program = ElpiProgram::Text(r#"
pred reports o:term, o:term.
reports X Y :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _.
reports X Z :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _, reports Y Z.
"#.into());
let r = db.now().elpi(&program, r#"reports (iri "urn:tiramemsu:v:alice") Y"#, &ElpiOptions::default())?;
// two rows: Y = bob and Y = carol
```

From the command line, with the program in a file `reporting.elpi`, run `tm elpi query mem.db reporting.elpi 'reports (iri "urn:tiramemsu:v:alice") Y'`. The bridge op is `elpi` and the MCP tool is `elpi_query`.

## Reading the store

A program never sees a copy of memory. It calls four host predicates, and tiramemsu answers each call from the view:

| Predicate | Reads |
|---|---|
| `tm.triple S P O E` | statements of the view; `E` is the statement as `(eid N)` |
| `tm.valid E From To` | the valid interval of statement `E`, bounds `(at Ms)` or `inf` |
| `tm.added S P O E` | statements the current transaction added (commit-time checks only) |
| `tm.retracted S P O E` | statements the current transaction retracted (commit-time checks only) |

A bound argument becomes an index lookup: `tm.triple (iri "urn:tiramemsu:v:alice") P O E` reads only alice's statements. A bound value that is not in the dictionary matches nothing and reads nothing. A run counts the rows these predicates return against a budget (`ElpiOptions::max_rows`, 10⁷ by default) and counts its resolution steps against `fuel`.

## Terms

ELPI is typed, so every tiramemsu value is a `term` built by one constructor of the prelude, which every program is loaded after:

| Value | Term |
|---|---|
| IRI | `(iri "urn:ex:a")` |
| string | `(str "Ann")` |
| integer | `(num 41)`, so `N` in `(num N)` is an ELPI `int` |
| double | `(dbl 1.5)` |
| boolean | `(bool "true")` |
| language-tagged string | `(lang "Hallo" "de")` |
| any other literal | `(lit "2024-01-02" "http://www.w3.org/2001/XMLSchema#date")` |
| node, blank node, transaction | `(node 3)`, `(bnode 4)`, `(tx 9)` |
| statement as a value | `(stmtv (eid 12))` |

Arithmetic works on the unwrapped value: `tm.triple _ (iri "urn:tiramemsu:v:age") (num N) _, M is N + 1` binds `M` to `42` for a stored age of 41. A solution's variables come back as values. A term that no value holds, such as a list or a lambda, comes back printed (`ElpiTerm::Other`).

## Time

Transaction time is the view's: on `db.as_of(TimeRef::Tx(t))` a program reads memory as it was believed then, and it runs the program text believed then too.

Valid time is pointwise. A goal query reads the statements valid at one instant: the view's `valid_at`, else `ElpiOptions::valid_at`, else now. A statement valid over `[2020, 2024)` is not read at 2025.

Conclusions and constraint checks read every statement and work out when their result holds. A conclusion holds over the intersection of its support's intervals. Premises valid over `[2020, 2025)` and `[2022, ∞)` give `[2022, 2025)`, and premises that were never valid together give nothing. A program that uses negation, cut, `findall` or `tm.retracted` is evaluated once per valid-time segment instead, because there the absence of a fact matters too.

## Constraints

A program states violations as clauses of `tm.violation`:

```
tm.violation error "one_ceo" "two CEOs" [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y), E1 = eid N1, E2 = eid N2, N1 < N2.
```

The arguments are the severity (`error` or `warn`), a name, a message and the statements the violation cites. `View::elpi_check` runs every `tm.violation` and reports each violation with the interval over which its statements hold together. Two CEOs whose terms never overlap are not a violation. The bridge op is `elpiCheck`, the MCP tool `elpi_check`, and the CLI command `tm elpi check`.

### Enforced at commit

`tx.enforce_elpi(program, true)` writes `(program elpi:enforced true)`. At that commit the full `tm.violation` runs, and an `error` violation fails the transaction with `ConstraintViolated`. On every later commit tiramemsu runs `tm.violation-delta Name Msg Eids` instead. In it, `tm.added` and `tm.retracted` read only what the transaction changed, so the check costs what the write costs:

```
tm.violation-delta "one_ceo" "two CEOs" [E1, E2] :-
  tm.added X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y).
```

You write the delta form, because ELPI programs cannot be unfolded automatically the way Datalog constraints can. A delta form that starts from `tm.retracted` catches violations caused by a removal, such as a project losing its only owner.

- A violation that the view as of the previous transaction already held does not fail the commit.
- A program without delta clauses is only checked when its text or its enforcement changes.
- Changing the text of an enforced program, or of a program it includes, runs the full check again.

The checks of one commit run under the constraint budget (`OpenOptions::constraint_budget`). The row cap is `max_seed_rows`, and the deadline becomes fuel at 4000 resolution steps per millisecond. A check that loops fails the transaction with `ConstraintBudgetExceeded` and writes nothing. A property test checks that an enforced program refuses a random transaction exactly when the full check finds a violation the transaction introduced.

## Stored programs

`tx.store_elpi_program(program, text)` loads the text to check it, then makes `(program elpi:source "<text>")` the node's one live source. New text supersedes the old version, and the same text writes nothing. A node holds rules of one language: storing ELPI on a node with N3 rules or a Datalog program, or the reverse, fails with `RuleLanguageConflict`.

A line `accumulate <iri>.` includes another stored program, read from the same view:

```
accumulate <urn:tiramemsu:v:lib>.
pred boss o:term.
boss Y :- reports (iri "urn:tiramemsu:v:alice") Y.
```

Includes nest. A cycle fails with `ProgramCycle` and a missing program with `UnknownProgram`. `View::elpi_program_info` lists the text, the source statement, every program included with its own source statement, whether the program may be non-monotonic, and whether it is enforced.

## Materialization

A program states conclusions as clauses of `tm.conclude`, listing the statements each one rests on:

```
tm.conclude (triple X (iri "urn:tiramemsu:v:grandparent") Z) [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:parent") Y E1,
  tm.triple Y (iri "urn:tiramemsu:v:parent") Z E2.
```

`View::elpi_conclude` returns the conclusions without writing. `Db::materialize_elpi(program, opts)` runs the stored program on a snapshot and writes each conclusion as a derived belief, in one short transaction.

- **Support.** A conclusion is supported by the statements it lists, the program's source statement and the source of every program it includes. It falls when a premise is retracted or when any of those programs is edited.
- **No forged support.** Every listed statement must be one the run's builtins returned, so a program cannot claim support it never read. An empty list is refused.
- **Re-runs.** Materializing again with no writes in between derives nothing. Each run retracts the program's earlier conclusions that it no longer derives, so a conclusion drawn from negation goes away once a new fact contradicts it.
- **Reruns after writes.** A program that may be non-monotonic is listed in `needs_rerun_after_writes`.

From the command line: `tm elpi program put mem.db urn:tiramemsu:v:g grand.elpi`, then `tm elpi materialize mem.db urn:tiramemsu:v:g`.

## Goals inside SPARQL and Cypher

An ELPI goal can be one part of a query, joined with ordinary patterns. It runs once per query, through the same path as Datalog goal calls:

```sparql
SELECT ?Y ?mail WHERE {
  SERVICE <urn:tiramemsu:elpi:query> {
    [] elpi:program v:reporting ; elpi:goal "reports (iri \"urn:tiramemsu:v:alice\") Y" }
  ?Y v:email ?mail }
```

```cypher
CALL tiramemsu.elpi.query('v:reporting', 'reports (iri "urn:tiramemsu:v:alice") Y') YIELD Y
MATCH (m) WHERE m = Y RETURN m.email
```

Each goal variable binds the query variable of the same name. `elpi:text "…"` (or `{text: "…"}` in Cypher) runs program text instead of a stored program. Two limits:

- A goal reads one valid instant, so `elpi:validFrom` and `elpi:validTo` stay unbound.
- A query row holds only stored terms, so a value the program computed and never stored fails the query with `Unsupported`. Use `View::elpi` for those.

## Every surface

| Surface | Calls |
|---|---|
| Rust | `View::elpi`, `elpi_check`, `elpi_conclude`, `elpi_program_info`; `TxElpi::store_elpi_program`, `enforce_elpi`; `Db::materialize_elpi` |
| CLI (`tm`, feature `elpi`) | `tm elpi query`, `check`, `conclude`, `materialize`, `program put\|show\|enforce\|unenforce` |
| JSON bridge (feature `elpi`) | `elpi`, `elpiCheck`, `elpiConclude`, `elpiProgramInfo`, `storeElpiProgram`, `enforceElpi`, `materializeElpi` |
| MCP (feature `elpi`) | `elpi_query`, `elpi_check`; write tools `store_elpi_program`, `enforce_elpi`, `materialize_elpi` |
| SPARQL, Cypher | `SERVICE <urn:tiramemsu:elpi:query>`, `CALL tiramemsu.elpi.query` |

## Errors

- `ElpiProgram`: the text does not parse, typecheck or load.
- `ElpiRun`: a runtime error, or support the run never read.
- `ElpiOutOfFuel`: the fuel or the row budget ran out.
- `ProgramCycle`: stored programs include each other.
- `UnknownProgram` and `RuleLanguageConflict`: as for Datalog.
- `ConstraintViolated` and `ConstraintBudgetExceeded`: at commit.

Without the feature every ELPI call fails with `Unsupported`.

## Limits

- lean-elpi is about 2.4 times slower than ELPI 1.18 (by geometric mean on ELPI's own benchmarks). No speed gate is set and no tiramemsu benchmark of ELPI exists yet.
- A host predicate returns all rows of one call at once, not lazily, so the row budget is what bounds memory.
- Programs choose no intervals: tiramemsu computes when a conclusion holds.
- There are no packages for Node, Python or the browser. lean-elpi has no Windows or wasm build.
