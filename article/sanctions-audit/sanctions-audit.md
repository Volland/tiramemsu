# What did the system know, and when?

*A worked example from a regulated domain: sanctions screening at a payments firm. Facts have two clocks and the screening rules are λProlog programs stored next to the facts, so every audit question becomes a query you can run again, years later, and get the same answer.*

---

> **Preview.** ELPI support in Tiramemsu is built and tested, but it is not in a release yet. The feature is called `elpi` and is off by default. Every program and query below is a test in `crates/tiramemsu/tests/elpi_sanctions.rs`. The people, companies and lists are invented, and the rules are simplified. This is an engineering example, not legal advice.

On a Tuesday in March a payments firm sends 48,000 euros to a shipping company. Two days later its sanctions feed delivers an update: the shipping company's ultimate owner was designated, and the designation had taken effect two days *before* the payment.

Three months later a supervisor asks four questions:

1. Did you pay a blocked party?
2. Did you know, or could you have known, when you paid?
3. Which rule did your screening apply, and who approved that rule?
4. What did you do once you found out, and when?

In many firms the answers come from forensics: a snapshot from the right night, an application log, a spreadsheet of rule changes, and whoever remembers how the screening service was configured in March. Each answer is a reconstruction, and a reconstruction can be challenged.

This article answers all four with queries. It uses [Tiramemsu](https://github.com/Volland/tiramemsu), an embedded graph database where every fact has an id and two clocks and nothing is ever deleted. The rules are [ELPI](https://github.com/Volland/lean-elpi) programs, a dialect of λProlog, stored in the same database as the facts. Then the article goes further than the supervisor's questions: a commit-time guard, a what-if exposure check, unit tests for the regulation itself, a false positive debugged back to the AI agent that caused it, and a policy change you can audit cell by cell.

## Two clocks, briefly

Every statement in Tiramemsu has two clocks.

- **Valid time** says when the fact held in the world. Marten Vale has been designated since 1 March.
- **Transaction time** says when the database learned the fact, and when (if ever) it stopped believing it. We recorded the designation on 5 March at 08:00.

Most databases keep at most one of these. If you overwrite rows you keep neither. If you keep an audit log you have transaction time but no valid time, and if you keep `valid_from` columns you have valid time but cannot say what you believed last month. Regulated work needs both, because the supervisor's first question is about the world and the second is about your knowledge of it.

Tiramemsu never deletes a row. A retraction closes a statement's transaction interval, and a correction (a *supersede*) retracts the old version and writes a new one linked to it. On SQLite, triggers in the database file enforce this. So `as_of(t)` returns exactly what was believed at `t`, and a property test checks that it equals a replay of the event log up to `t`.

## The cast

The firm keeps one Tiramemsu database. Four writers feed it, and every write is a transaction that records who made it and from what:

- **The registry feed** writes company ownership from filings: who owns what share of whom, and since when.
- **The sanctions feed** writes designations from the published lists.
- **An extraction agent** reads filings that are not machine-readable. It is a language model, so it is useful and it makes mistakes.
- **The payments service** screens each payment and releases it.

The ownership graph is small. Marten Vale owns 55% of Brightwater Holdings, Brightwater owns 60% of Halcyon Shipping, and Vale also owns exactly 50% of Corvid Marine.

A share is a statement *about* an ownership statement. In Tiramemsu a statement has an id, so it can be the subject of another statement. This is what the docs call a layer:

```rust
db.transact(TxOptions::default(), |tx| {
    tx.meta(sys_author, v("registry-feed"))?;
    tx.meta(sys_source, Value::str("filing-7731"))?;
    let e = tx.assert(v("vale"), v("owns"), v("brightwater"), Valid::from(june_2025))?.eid();
    tx.assert(e, v("share"), Value::Int(55), Valid::from(june_2025))?;
    // … Brightwater owns 60% of Halcyon, Vale owns 50% of Corvid
    Ok(())
})?;
```

`tx.meta` writes ordinary triples whose subject is the transaction. Nothing on the fact itself says who wrote it. The transaction says it once, for everything it wrote.

## The rule is data

Compliance writes the screening policy as an ELPI program and stores it in the database:

```
pred designated o:term, o:list stmt.
designated X [E] :- tm.triple X (iri "urn:tiramemsu:v:designatedBy") _ E.

pred owns o:term, o:term, o:int, o:list stmt.
owns X Y P [E1, E2] :-
  tm.triple X (iri "urn:tiramemsu:v:owns") Y E1,
  tm.triple (stmtv E1) (iri "urn:tiramemsu:v:share") (num P) E2.

pred controlling o:int.
controlling P :- P > 50.

pred blocked o:term, o:list stmt.
blocked X Es :- designated X Es.
blocked Y Es :- owns X Y P Own, controlling P, blocked X Up, std.append Own Up Es.
```

Read it from the bottom. A party is blocked if it is designated, or if a blocked party owns a controlling share of it. The rule is recursive, so it follows ownership chains of any length.

Three details matter for what follows.

- **The rule reads memory directly.** `tm.triple S P O E` is a host predicate, so each call is an index lookup on a view of the database. The program never gets a copy of the data. On an as-of view it reads memory as it was then.
- **The rule reads layers.** `tm.triple (stmtv E1) … share …` reads the share attached to the ownership statement `E1`. The percentage belongs to that ownership fact, not to the company.
- **The rule cites its evidence.** Each predicate returns the list of statements it used, `Es`. In a regulated system an answer without its evidence is not much use.

The policy is stored like any other fact:

```rust
db.transact(TxOptions::default(), |tx| {
    tx.meta(sys_author, v("compliance"))?;
    tx.meta(sys_reason, Value::str("screening policy v1"))?;
    tx.store_elpi_program(v("screening"), SCREENING_V1)?;
    Ok(())
})?;
```

The program text becomes one statement, `(screening elpi:source "<text>")`. It has an id, an author, a reason and both clocks. If you store new text, the old version is superseded, not overwritten. Three more programs include the policy with `accumulate <urn:tiramemsu:v:screening>`: a guard, a look-back and a blocklist. We will meet each of them.

## 3 March: the decision

At 10:00 the payments service screens pay1042 to Halcyon:

```rust
let r = db.now().elpi(
    &ElpiProgram::Stored(v("screening")),
    r#"blocked (iri "urn:tiramemsu:v:halcyon") Es"#,
    &ElpiOptions { valid_at: Some(march_3_10am), ..ElpiOptions::default() },
)?;
// no rows: nothing makes Halcyon blocked
```

There are no solutions, so the payment is released. The release is one transaction that says who released it and why:

```rust
db.transact(TxOptions::default(), |tx| {
    tx.meta(sys_author, v("payments"))?;
    tx.meta(sys_reason, Value::str("screened clear"))?;
    tx.assert(v("pay1042"), v("paidTo"), v("halcyon"), Valid::between(t0, t0 + 1))?;
    Ok(())
})?;
```

That is the whole decision record. It does not copy the rule into the record, because the rule is a statement in the same store. The view as of the release's transaction holds the exact program text that screened it:

```rust
let rule = db.as_of(TimeRef::Tx(release_t)).elpi_program_info(&v("screening"))?;
assert_eq!(rule.text, SCREENING_V1);
```

The supervisor's third question, "which rule did you apply?", is already answered. The program's transaction answers "who approved it": `compliance`, with the reason "screening policy v1".

## 5 March: the list arrives

At 08:00 on 5 March the sanctions feed writes the designation, valid from 1 March:

```rust
db.transact(TxOptions::default(), |tx| {
    tx.meta(sys_author, v("sanctions-feed"))?;
    tx.meta(sys_source, Value::str("eu-list-2026-041"))?;
    tx.assert(v("vale"), v("designatedBy"), v("eu-list"), Valid::from(march_1))?;
    Ok(())
})?;
```

Now ask the same screening question on three views.

![A timeline in two rows. Top row, the world: Vale is designated from 1 March, pay0977 is on 10 February and pay1042 on 3 March. Bottom row, the database: the release is recorded on 3 March and the list on 5 March, 104 hours after the designation took effect. On the right, three answers: as of 3 March, valid on 3 March, clear; now, valid on 3 March, blocked with five cited facts; now, valid on 15 February, clear.](images/01-two-clocks.png)

*One question on three views. The transaction clock picks what we knew, the valid clock picks the day we ask about.*

```rust
// 1. what we knew when we decided: clear
let then = db.as_of(TimeRef::Instant(march_3_10am));
screen(&then, "halcyon", march_3_10am);          // no rows

// 2. what we know now about that day: blocked
screen(&db.now(), "halcyon", march_3_10am);      // Es = [eid 5, eid 6, eid 3, eid 4, eid 21]

// 3. what we know now about February: clear, the designation was not in force
screen(&db.now(), "halcyon", feb_15);            // no rows
```

So the answer to "did you pay a blocked party?" is yes. The payment fell inside the designation period. The answer to "did you know?" is no, and it can be shown: on the view as of the decision, the designation does not exist.

The as-of run also uses the program text that was live then. If compliance had changed the rule since, the replay would still run the March rule. Nobody has to rebuild a past deployment.

The second answer cites five statements. These are the reasons Halcyon is blocked, nearest owner first:

![The ownership chain: Marten Vale, designated by the EU list (e21, valid from 1 March, recorded 5 March), owns Brightwater Holdings (e3, with layer e4: share 55), which owns Halcyon Shipping (e5, with layer e6: share 60). pay1042 points to Halcyon. A dashed edge e7 with share 50 (e8) goes from Vale to Corvid Marine: exactly 50%, not control under rule v1. Below, the evidence list Es = [eid 5, eid 6, eid 3, eid 4, eid 21].](images/02-evidence.png)

*The evidence is a list of statement ids, and each id leads to an author, a source and both clocks.*

Every id in that list is a handle. You can look up who wrote the statement, from which filing, when it became true, when it was recorded, and whether anything has superseded it since.

## When did we learn it?

The gap between the two clocks is a metric in its own right. In SPARQL, `tm:validFrom` and `tm:addedAt` are virtual predicates on any statement:

```sparql
# facts recorded after they had already become true
SELECT ?who ?from ?added WHERE {
  ?who v:designatedBy ?list ~ ?r .
  ?r tm:validFrom ?from ; tm:addedAt ?added
  FILTER(?added > ?from) }
```

In Cypher the same gap comes out in hours, with the source taken from the transaction that wrote it:

```cypher
MATCH (x)-[r:designatedBy]->(l)
RETURN x,
       (r.addedAt.epochMillis - r.validFrom.epochMillis) / 3600000 AS hoursLate,
       tiramemsu.transaction(r).`sys:source` AS source
```

The answer is Vale, 104 hours, `eu-list-2026-041`. Run it per feed and per month and you have the latency report a supervisor asks for, as a query rather than a project. Only a store that keeps both clocks on every statement can ask it.

## The look-back: what we now know about the past

On 5 March the firm must also find every past payment that went to a party it now knows was blocked at the time. That is a full check over history, written as a warning:

```
accumulate <urn:tiramemsu:v:screening>.
tm.violation warn "lookback" "paid a party that was blocked at the time" [E | Es] :-
  tm.triple _ (iri "urn:tiramemsu:v:paidTo") B E, blocked B Es.
```

`View::elpi_check` runs it and returns one violation: pay1042. A February payment to Halcyon, pay0977, is not reported. The rule never mentions dates, so why not?

A violation holds over the intersection of the valid intervals of the statements it cites. pay0977 is valid on 10 February and the designation from 1 March, so the two never overlap and there is no violation. pay1042's instant falls inside the designation, so it is reported, and the violation carries that instant as its interval. Tiramemsu does the temporal reasoning, and the rule only states what is wrong.

Before 5 March the same check returns nothing, and it still does on any view as of an earlier transaction. So you can show when the firm could first have found the problem, as well as when it did.

## The guard: refuse the next one

The look-back works on the past. For new payments the firm wants a guard that refuses a payment to a blocked party before it is written:

```
accumulate <urn:tiramemsu:v:screening>.
tm.violation-delta "sanctions" "payment to a blocked party" [E | Es] :-
  tm.added _ (iri "urn:tiramemsu:v:paidTo") B E, blocked B Es.
```

```rust
db.transact(TxOptions::default(), |tx| tx.enforce_elpi(v("guard"), true))?;
```

On 20 March a second payment to Halcyon is attempted. The commit fails with `ConstraintViolated`. The error names the guard and cites the payment, the designation and both ownership statements with their shares, and it carries the interval over which the violation holds. Nothing is written, not even a transaction row. A payment to an unrelated supplier five minutes later commits normally.

The guard has a delta form only, and that choice matters. `tm.added` reads only what the current transaction adds, so the check costs what the write costs, not what the database holds. A full form would also run whenever the guard's text changes. It would then find pay1042, a historical payment to a now-blocked party, and refuse to register the guard at all. Worse, a guard seeded by an added designation would refuse the sanctions feed's own write on 5 March, because that write is what makes pay1042 a violation. So the guard looks forward and the look-back looks back. They share one policy through `accumulate`, and each has its own job.

A refused write leaves no trace in the database. If you must keep refused attempts, record each refusal in its own transaction. The error already carries everything you need: the guard, the message and the cited ids.

## What if: exposure before it happens

Compliance hears that Brightwater Holdings may be listed next month. Which counterparties would that block? λProlog can answer under an assumption, with `=>`:

```
designated (iri "urn:tiramemsu:v:brightwater") [] => blocked C _
```

`=>` adds the clause `designated brightwater []` for this one goal. The answer is Brightwater and Halcyon. Nothing is written: the database's last transaction number is the same before and after. A speculative transaction rolled back would give the same answer, but it would hold the single writer while it ran. Here the hypothesis is part of the question, and it runs on any read view, past ones included.

You can ask the same question for every name on a watch list, every morning. It also answers "what would this ownership change do?" when an acquisition is announced before it closes.

## Test the rule, not just the data

The rule is code, and in a regulated firm code needs tests. A test over the current data only shows that the rule fits today's graph. λProlog can test it against any owner and any company, using `pi`:

```
% a designated owner with 51% blocks the company
pi x\ pi y\ (designated x [], owns x y 51 []) => blocked y _

% control passes along a chain
pi x\ pi y\ pi z\ (designated x [], owns x y 80 [], owns y z 80 []) => blocked z _

% exactly 50%: the line between v1 and v2
pi x\ pi y\ (designated x [], owns x y 50 []) => blocked y _
```

`pi x\` introduces a name that occurs nowhere in the data. So a success means the rule holds for an arbitrary designated owner, not for Vale in particular. Under policy v1 the first two goals succeed and the third fails, because exactly 50% is not more than 50%. These goals are ordinary test cases. Run them in CI against every proposed version of the policy, and a reviewer can see from the failing property what a rule change does before it reaches a payment.

This is the verification layer. The facts are checked against the rule at commit, and the rule is checked against the regulation's intent in CI.

## Debugging a false positive

On 28 March the extraction agent reads a filing for Vale Logistics Ltd, an unrelated company with an unlucky name. It writes that `vale` owns 70% of Harbor Freight. Its transaction records the agent, the filing and the model version:

```rust
tx.meta(sys_author, v("registry-extractor"))?;
tx.meta(sys_source, Value::str("filing-9132"))?;
tx.meta(v("modelVersion"), Value::str("2.3"))?;
```

On 30 March the nightly job materializes the blocklist. That program states blocked status as conclusions with their support:

```
accumulate <urn:tiramemsu:v:screening>.
tm.conclude (triple X (iri "urn:tiramemsu:v:blocked") (bool "true")) Es :- blocked X Es.
```

Harbor Freight now appears on the blocklist. On 2 April at 10:00 a routine payment to Harbor Freight is refused. Here is how the analyst on duty traces it.

**1. Read the refusal.** The error cites statement ids, and one of them is `vale owns harbor`.

**2. Ask who wrote that statement.**

```sparql
SELECT ?who ?src ?model WHERE {
  v:vale v:owns v:harbor ~ ?r . ?r tm:txAdded ?t .
  ?t sys:author ?who ; sys:source ?src ; v:modelVersion ?model }
```

`~ ?r` binds the statement itself, and `tm:txAdded` steps to the transaction that wrote it.

The answer is `registry-extractor`, `filing-9132`, `2.3`.

**3. Ask what else that model version wrote.** This is the blast radius:

```sparql
SELECT ?s ?p ?o WHERE {
  ?s ?p ?o ~ ?r . ?r tm:txAdded ?t .
  ?t v:modelVersion "2.3" FILTER(?s != ?t) }
```

In this example the answer is the one ownership and its share. In production it is the list of everything to review before you trust that model version again.

**4. Ask what the fix would take with it.** `View::dependents(eid)` walks the same cascade a retraction would, without taking the writer lock. It returns the ownership, its share, the materialized `harbor blocked true` belief and the support links of that belief.

**5. Fix it in one transaction that says why.**

```rust
db.transact(TxOptions::default(), |tx| {
    tx.meta(sys_author, v("analyst-kim"))?;
    tx.meta(sys_reason, Value::str("entity resolution: Vale Logistics Ltd is not Marten Vale"))?;
    tx.retract(wrong)?;
    stake(tx, "vale-logistics", "harbor", 70, jan_1)?;
    Ok(())
})?;
```

The derived belief that Harbor Freight is blocked falls in the same transaction, because its support is gone. No one has to remember to clean up the blocklist. The payment is retried at 14:05 and commits.

**6. Reproduce the bug afterwards.** The screening as of 2 April 10:00 still says Harbor Freight is blocked, and on the current view it is clear. The bug can be reproduced exactly, after the fix, as often as a reviewer wants. Over history, a question shows why the fact was retracted, by whom, and when:

```sparql
SELECT ?x ?y ?who ?why FROM <urn:tiramemsu:tm:history> WHERE {
  ?x v:owns ?y ~ ?r . ?r tm:txRetracted ?t .
  ?t sys:author ?who ; sys:reason ?why }
```

The answer is `vale`, `harbor`, `analyst-kim`, "entity resolution: Vale Logistics Ltd is not Marten Vale".

This is debuggability in a precise sense. The state that produced the bad decision is still there. You do not reconstruct it from logs, you open a view on it.

## The rule changed

On 4 May the firm starts clearing US dollars. EU and UK guidance treat ownership of *more than* 50% as control. The US rule, OFAC's, covers parties owned 50% *or more* by blocked persons, in aggregate. Compliance moves to the stricter line, and the change is one line of the program:

```
controlling P :- P >= 50.
```

```rust
db.transact(TxOptions::default(), |tx| {
    tx.meta(sys_author, v("compliance"))?;
    tx.meta(sys_reason, Value::str("policy v2: 50% or more, for USD clearing"))?;
    tx.store_elpi_program(v("screening"), SCREENING_V2)?;
    Ok(())
})?;
```

Three things happen with no further code.

**The old conclusions fall with the old rule.** A materialized conclusion is supported by its premises *and* by the source statements of the program and every program it includes. Storing v2 supersedes the v1 source, so every blocklist entry derived under v1 is retracted in the same transaction. Rematerializing derives four entries: Vale, Brightwater, Halcyon and, newly, Corvid Marine. A query that compares the current blocklist with the one as of 1 May returns only the change:

```sparql
SELECT ?x WHERE {
  ?x v:blocked true .
  OPTIONAL { SERVICE <urn:tiramemsu:tm:asOf/2026-05-01T00:00:00Z> { ?x v:blocked ?then } }
  FILTER(!BOUND(?then)) }
```

It returns `corvid`.

**The look-back now flags a payment that was compliant when it was made.** pay1100 to Corvid was screened clear on 20 April under v1. Whether the firm has to remediate it is a compliance decision. The database's job is to make the facts behind that decision unambiguous, so you replay the decision four ways:

![A two-by-two grid for pay1100 to Corvid Marine. Rows: rule v1 (more than 50%) and rule v2 (50% or more). Columns: facts as known on 20 April and facts as known today. Rule v1 gives clear in both columns; rule v2 gives blocked in both. The cell for rule v1 and the facts of 20 April is outlined: the decision as it was taken.](images/03-replay.png)

*Rule versions against states of knowledge. Every cell is one call.*

- **v1 on what we knew on 20 April:** clear. This is the decision as it was taken: the as-of view runs the stored program, which was v1 then.
- **v1 on what we know today:** clear. The v1 text comes from the as-of view's `elpi_program_info`, run on the current view.
- **v2 on what we knew on 20 April:** blocked.
- **v2 on what we know today:** blocked. This is today's answer.

The outcome follows the row, not the column. The change came from the rule, not from new facts, so you can tell the supervisor the payment was compliant under the policy in force, and show when and why the policy changed. The policy's own history is a query too: two versions, both by `compliance`, with their reasons.

## The audit pack

Put together, here is what the firm can hand a supervisor. Each item is a query or a call, not a reconstruction:

- **What was true:** the screening goal on the current view at the payment's valid instant, with its cited statements.
- **What we knew:** the same goal on the view as of the decision's transaction, run with the program text that was live then.
- **When we learned it:** `tm:addedAt` against `tm:validFrom`, per statement, per feed.
- **Who said so, and from what:** the author, source and model version on the transaction of every cited statement.
- **What we did about it:** the look-back's violations, and the transactions that followed, with their reasons.
- **Which rule, approved by whom:** `elpi_program_info` as of any transaction, and the transaction that stored that text.
- **What a change would do:** `=>` goals for proposed designations, `pi` properties for proposed rule versions, `dependents` for proposed corrections.
- **That nothing was quietly changed:** no code path deletes a row, SQLite triggers enforce it, and `as_of(t)` equals the replay of the event log up to `t` for every `t`, checked by property tests.

## The same shape in a hospital

Nothing above is specific to payments. Swap the nouns and you have clinical decision support:

- A **designation** becomes an allergy or a diagnosis. Its valid time is the onset, and it is often recorded days later.
- The **ownership chain** becomes a drug-class hierarchy: an allergy to penicillins covers amoxicillin.
- The **guard** refuses a prescription that conflicts with a recorded allergy, at commit.
- The **look-back** finds patients who were prescribed a drug that falls under an allergy recorded afterwards.
- **What if** asks which current prescriptions need review if a patient's kidney function falls below a threshold.
- **Rule versions** are guideline and formulary updates. "Which guideline applied to this order?" is an as-of call.
- The **false positive** is an AI scribe that attached a note to the wrong patient, traced to its model version and fixed in one transaction with a reason.

The questions a clinical incident review asks are the supervisor's four questions: what was true, what was known, which rule applied, what happened next.

## Limits

- **Preview.** ELPI is built and tested in Tiramemsu but not released. It runs from Rust, the `tm` command line and the MCP server, on macOS and Linux. The Node, Python and browser packages do not include it yet.
- **Simplified rule.** The US rule adds up stakes held by several blocked owners. Summing needs `std.findall`, which makes a program non-monotonic: Tiramemsu supports that, but this example leaves it out. Cross-holdings form cycles, so a production rule should carry a visited list. Until it does, the run's fuel budget stops a loop with an error instead of a hang.
- **Cite rules by transaction, not by statement.** A statement whose object is another statement moves with it when that statement is superseded. If a decision's transaction points at the policy's source statement, then after v2 is stored the current view shows the decision pointing at v2. History still holds the original link. That is why this example finds the rule through the as-of view of the decision's transaction, which cannot drift. A test in the suite pins this behaviour.
- **Refused writes are not stored.** Record them yourself if you need them.
- **Erasure is not built yet.** Never deleting conflicts with a data subject's right to erasure. The design is crypto-shredding: sensitive literals are sealed with a per-subject key, and erasing destroys the key. It is planned but not implemented. Identifiers will never be sealed, so personal data that may need erasing belongs in literals, not in node IRIs.
- **Speed is not measured yet.** lean-elpi is about 2.4 times slower than the reference ELPI 1.18, and there is no Tiramemsu benchmark of ELPI yet. Delta guards cost what the write costs. Full checks cost what the data costs.

## Try it

The whole story runs as one test file:

```sh
LEANELPI_DIST=/path/to/lean-elpi/dist \
  cargo test -p tiramemsu --features elpi --test elpi_sanctions
```

Ten tests, one per section: the decision record, two clocks and three answers, when we learned it, the look-back, the guard, what if, the rule properties, the false positive, the rule change, and the citation limit above. The [ELPI guide](https://volland.github.io/tiramemsu/elpi/) covers every call. *Validators that ask "what if"* covers `=>`, `pi` and commit guards in more depth, and *λProlog in a bitemporal memory* tells the engineering story behind them.

A regulator's questions are about time: what was true, what you knew, and when. If your database has only one clock, answering them is forensic work. With two clocks and the rules in the same store, each answer is a query you can run again.
