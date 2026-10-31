# Validators that ask "what if"

*A hands-on tour of ELPI in Tiramemsu: questions over memory, hypothetical answers that change nothing, policies passed around as data, guards that refuse a bad write, and conclusions that stay grounded. Every example below is a test in the repository.*

---

> **Preview.** ELPI support is built and tested but not released. It needs lean-elpi 0.4, whose host callbacks are not on crates.io yet, and it will ship in a later Tiramemsu release. The feature is called `elpi`, and it is off by default.

An agent's memory is full of rules nobody wrote down: one CEO at a time, every project has an owner, a report needs a reviewer who is not its author. Tiramemsu can check rules like these in Datalog. Some questions, though, are not about what is true but about what would be true if something changed. Would the team still reach Erin if Bob left? Does this policy hold for any new member, whoever that is? Which of these ten policies does this project break, when the policies themselves are data?

These are λProlog's home ground, and ELPI is the dialect of λProlog that Tiramemsu runs, through lean-elpi. This article walks through six small programs, from a plain question to a guard that runs on every write.

## Setup

Build with the feature `elpi` and point `LEANELPI_DIST` at the directory that holds `libleanelpi`. The examples use this team:

```text
alice → bob → dana → erin
alice → carol → dana
```

Each arrow is a `v:manager` statement: alice manages bob and carol, both of them manage dana, and dana manages erin.

## 1. Ask a question

A program reads memory with `tm.triple S P O E`. `E` is the statement as `(eid N)`, and values are wrapped by type: IRIs in `(iri "…")`, strings in `(str "…")`, integers in `(num N)`.

```
pred reaches o:term, o:term.
reaches X Y :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _.
reaches X Z :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _, reaches Y Z.
```

```rust
let r = db.now().elpi(&program, r#"reaches (iri "urn:tiramemsu:v:alice") Y"#, &ElpiOptions::default())?;
```

The goal returns bob, carol, dana and erin. Dana and erin appear twice, once through bob and once through carol, because the search finds each path. The program never sees a copy of memory: each `tm.triple` call is an index lookup on the view, and an as-of view runs it over memory as it was then.

## 2. Ask what would happen

Add one predicate that no clause defines, and let the rules respect it:

```
pred gone o:term.
reaches X Y :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _, not (gone Y).
reaches X Z :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _, not (gone Y), reaches Y Z.
```

Now ask the question under an assumption:

```
gone (iri "urn:tiramemsu:v:bob") => reaches (iri "urn:tiramemsu:v:alice") Y
```

`=>` adds the clause `gone bob` for this one goal. Alice still reaches carol, dana and erin, so losing bob breaks nothing. Assume both are gone:

```
(gone (iri "urn:tiramemsu:v:bob"), gone (iri "urn:tiramemsu:v:carol")) => reaches (iri "urn:tiramemsu:v:alice") Y
```

There is no answer: without bob and carol, alice reaches nobody. Nothing was written or retracted. The next plain query returns the full chain again, and the store's history shows no write. In SPARQL or Datalog the usual way is a speculative transaction that is rolled back. Here the hypothesis is part of the question.

## 3. Policies are data

In λProlog a predicate can be an argument. So a policy can be a named check that the validator applies, and a new policy is one more clause, not a new rule:

```
pred policy o:string, o:(term -> prop).
policy "has an owner"   (P\ tm.triple P (iri "urn:tiramemsu:v:owner") _ _).
policy "has a due date" (P\ tm.triple P (iri "urn:tiramemsu:v:due") _ _).

tm.violation warn Name "a project breaks a policy" [E] :-
  tm.triple P (iri "urn:tiramemsu:v:type") (iri "urn:tiramemsu:v:Project") E,
  policy Name Check, not (Check P).
```

`P\ …` is a lambda, and `Check P` applies it. Take two projects: p1 has an owner but no due date, and p2 has a due date but no owner. Then `View::elpi_check` reports two `warn` violations, "has a due date" and "has an owner". Each one cites the project's type statement and carries the time over which that statement holds. Storing the policies as an ELPI program makes them versioned like any other statement: edit one, and the next check uses the new text.

## 4. For any member at all

`pi x\` introduces a name that exists nowhere else. Together with `=>` it checks that a rule holds for any member, not just the ones in memory:

```
pred member o:term, o:term.
pred staffed o:term.
staffed T :- member _ T.

pred rule_ok.
rule_ok :- pi x\ member x (iri "urn:tiramemsu:v:fresh") => staffed (iri "urn:tiramemsu:v:fresh").
```

`rule_ok` succeeds: whoever the new member is, a team with that member counts as staffed. This is the shape of a schema or contract check. You test the rule against an arbitrary member rather than a sample.

## 5. Refuse a bad write

A validator can guard every commit. It states its violation twice: a full form for the first check, and a delta form that starts from what the transaction added.

```
tm.violation error "one_ceo" "two CEOs" [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y), E1 = eid N1, E2 = eid N2, N1 < N2.

tm.violation-delta "one_ceo" "two CEOs" [E1, E2] :-
  tm.added X (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E1,
  tm.triple Y (iri "urn:tiramemsu:v:role") (iri "urn:tiramemsu:v:CEO") E2,
  not (X = Y).
```

```rust
db.transact(TxOptions::default(), |tx| {
    tx.store_elpi_program(v("policy"), POLICY)?;
    tx.enforce_elpi(v("policy"), true)?;
    Ok(())
})?;
```

From then on, a transaction that asserts a second CEO fails with `ConstraintViolated` and writes nothing, while one that only renames someone commits. A delta form can also start from `tm.retracted`, which catches a project losing its only owner.

Each check runs under the commit's budget, and a validator that loops fails the write instead of holding the writer. A property test offers random histories to an enforced database and to an unenforced copy. It checks that a write is refused exactly when the full check on the copy finds a violation the write introduced.

## 6. Remember what follows

`tm.conclude` states a conclusion and the statements it rests on:

```
tm.conclude (triple X (iri "urn:tiramemsu:v:grandmanager") Z) [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:manager") Y E1,
  tm.triple Y (iri "urn:tiramemsu:v:manager") Z E2.
```

`Db::materialize_elpi` writes each conclusion as a derived belief, supported by the listed statements, by the program and by any library it includes. Retract `bob manager dana` and alice stops being dana's grandmanager through bob. Edit the program, and all its conclusions go. A program cannot claim support it never read, because the run refuses any statement id its builtins did not return. Running it again with nothing changed writes nothing.

## Inside a query

A goal can be one part of a SPARQL or Cypher query, so ELPI answers join ordinary patterns:

```sparql
SELECT ?Y ?mail WHERE {
  SERVICE <urn:tiramemsu:elpi:query> {
    [] elpi:program v:reporting ; elpi:goal "reports (iri \"urn:tiramemsu:v:alice\") Y" }
  ?Y v:email ?mail }
```

## Which language when

- **Datalog:** use it for recursion and aggregates at scale, and for rules over valid time with its temporal operators. It is the fast path and it enforces constraints by itself.
- **N3:** use it when the rules come as N3, or when you want proofs in N3's own vocabulary.
- **ELPI:** use it when the rule needs search, an assumption (`=>`), a fresh name (`pi`) or a rule as an argument. This is the case for validators and contracts.

All three store their programs the same way, read the same views and write conclusions that fall with their premises. A node holds programs of one language only.

## Limits

ELPI is in preview. lean-elpi is about 2.4 times slower than ELPI 1.18, and Tiramemsu has no ELPI benchmark yet. The native library exists for macOS and Linux only, so the Node, Python and browser packages leave ELPI out. The command line, the MCP server and Rust have it.

The guide has every call, term and error. The engineering story, from callbacks to the bugs the tests found, is the companion article *λProlog in a bitemporal memory*.
