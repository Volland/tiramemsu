# Article: What did the system know, and when?

A Substack long read (about 4,400 words, 3 illustrations) on Tiramemsu in a regulated domain: sanctions screening at a payments firm. It covers both clocks, rules stored as ELPI programs, the decision record, a look-back, a commit guard, what-if exposure with `=>`, rule properties with `pi`, a false positive debugged back to the agent that caused it, and a policy change replayed against both rule versions. A short section maps the same shape to a hospital. It is marked **Preview** until ELPI ships in a release.

| File | What it is |
|---|---|
| `sanctions-audit.md` | The article, the source of truth |
| `preview.html` | Self-contained preview with the images embedded: open it, select all, paste into Substack |
| `build-preview.sh` | Rebuilds `preview.html` (needs `pandoc`; uses `../preview.css`) |
| `images/*.png` | The illustrations at 1456 px wide |
| `images/src/*.svg` | Their editable sources, in the tiramisu palette |

Re-render the images after editing an SVG:

```sh
cd images && for f in src/*.svg; do rsvg-convert -w 1456 "$f" -o "$(basename "$f" .svg).png"; done
```

## Where the claims come from

Every program, query and result in the article is a test in `crates/tiramemsu/tests/elpi_sanctions.rs`, one per section:

- "3 March: the decision": `decision_record_and_the_rule_in_force`
- "5 March: the list arrives": `two_clocks_three_answers` (pins the evidence list `[eid 5, eid 6, eid 3, eid 4, eid 21]`)
- "When did we learn it?": `when_we_learned_it` (104 hours, the source)
- "The look-back": `lookback_finds_the_payment`
- "The guard": `guard_refuses_at_commit`
- "What if": `what_if_brightwater_were_listed`
- "Test the rule, not just the data": `rule_properties_per_version`
- "Debugging a false positive": `debugging_a_false_positive`
- "The rule changed" and the replay grid: `policy_change_is_auditable`
- "Limits", citing rules by statement: `citation_by_statement_follows_supersede`

Run them with `LEANELPI_DIST=… cargo test -p tiramemsu --features elpi --test elpi_sanctions`. The Rust snippets in the article are trimmed from that file (helper names such as `sys_author` stand for `Value::iri(vocab::SYS_AUTHOR)`). A change to the ELPI prelude, the builtins, the temporal syntax or supersede means re-running them and updating the text.

The regulatory framing (EU and UK: more than 50%; US OFAC: 50% or more, in aggregate) is simplified and is not legal advice. The names, companies and lists are invented.

## Publishing on Substack

1. **Title:** What did the system know, and when?
2. **Subtitle:** Sanctions screening with two clocks and rules stored as data: an audit trail you can replay, in Tiramemsu with ELPI.
3. **Cover / social image:** `images/01-two-clocks.png`.
4. **Body:** run `./build-preview.sh`, paste from `preview.html`, then delete the title and the italic standfirst if you put them in Substack's own fields. Check that code blocks came through as code blocks; upload any image that did not paste.
5. **Captions and alt text:** the italic line under each image is its caption, and each image's alt text is in the Markdown.
6. **Before publishing:** check the Preview banner still applies, and that the guide link `volland.github.io/tiramemsu/elpi/` is live.

Substack has no tables, so the article uses lists.

### Other title options

- Two clocks and a rulebook: an audit trail you can replay
- The supervisor's four questions, answered with queries
- Debugging compliance: time travel for regulated decisions

### SEO description (under 160 characters)

Sanctions screening with two clocks and rules as data: replay any decision as it was taken, cite its evidence, and audit every rule change.

### LinkedIn post

> A payment goes out on Tuesday.
> On Thursday your sanctions feed tells you the payee's owner was designated on Sunday.
>
> The supervisor asks four questions:
> 1. Did you pay a blocked party?
> 2. Did you know when you paid?
> 3. Which rule did you apply, and who approved it?
> 4. What did you do once you found out?
>
> In most stacks, each answer is a forensic job: snapshots, logs, and someone's memory of what was deployed in March.
>
> I wrote up how Tiramemsu turns each one into a query.
>
> Tiramemsu is an embedded graph database I'm building for memory that has to be accountable:
> 🔹 every fact has an id, so an answer can cite exactly what it used
> 🔹 every fact has two clocks: when it was true in the world, and when we learned it
> 🔹 nothing is deleted: a correction is a new version, and the database can be read exactly as it stood at any past moment
> 🔹 every write is a transaction that records who wrote it, from which source and why, whether the writer was a feed, an analyst or an AI agent
>
> And the screening rules are ELPI (λProlog) programs stored in the same database as the facts. So a rule has versions, an author and a history, and replaying a decision runs the rule that was in force at the time.
>
> The article walks through a worked case: a commit-time guard that refuses a bad payment, a look-back over past payments, "what if Brightwater gets listed next month?", unit tests for the regulation itself, a false positive traced back to the model version of the agent that caused it, and a policy change from "more than 50%" to "50% or more" replayed against both versions.
>
> Every query in it is a test in the repo. The same pattern applies to clinical decision support, with an allergy that is recorded late.
>
> ELPI support is a preview: built and tested, not released yet.
>
> 🔗 [link]
>
> #RegTech #Compliance #KnowledgeGraphs #Bitemporal #AIAgents #Rust

### Substack note

> "Did you know when you paid?" is a question about time, and most databases keep only one clock.
>
> New piece: sanctions screening on Tiramemsu. Every fact carries when it was true and when we learned it, nothing is ever deleted, and the screening rules are λProlog programs stored next to the facts. You can replay any decision exactly as it was taken, with the rule version that made it and the facts it cited.
>
> Also inside: a commit guard, what-if exposure, unit tests for a regulation, and a false positive traced back to an AI agent's model version.
>
> [link]

### Short social post

> A payment goes out on Tuesday. On Thursday you learn the payee's owner was sanctioned from Sunday. Did you breach? Did you know? Which rule did you apply? With two clocks on every fact and the rules stored next to the facts, each answer is one query, and you can run it again years later.
