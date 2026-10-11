# Project Review

Two reviews record measured behavior and proposed work without changing the implemented architecture. Reports live in `docs/`; the first review's probes are in `docs/review-results/`.

## Second Review

The 2026-10-10 review of 0.4.0 is in `docs/project-review.md`, with the proposed releases in `docs/feature-roadmap.md`. It scores the project 8/10 overall and finds no data-loss bugs.

Its high-priority items are evidence at scale (the scale ceiling in [[overview#Open Inputs]] is still unset and the as-of churn target of [[roadmap#Benchmarks]] is unmet), release legibility (no CHANGELOG, npm unpublished) and missing Cypher temporal types. Medium items include row-by-row cascade loading in supersede, term-cache clearing on large transactions, a spinning bounded writer lock, unbounded LFTJ input materialisation, SPARQL decimal and language-tag deviations, and the Lean store model at the repository root, which cannot build as laid out.

Most 0.4.x items were fixed the same day. Isolating the history-depth slowdown also found that STAT4 re-prepared every fixed-plan lookup, fixed with `+?N` parameters ([[query#Physical Planning#Join Ordering]], [[tests#Review Fixes]]).

The roadmap orders work as 0.4.x hardening, 0.5 scale evidence, 0.6 hybrid recall, 0.7 temporal types, 0.8 crypto-shredding, 0.9 memory merge and 1.0 stability ([[roadmap#Milestones]]).

## First Review

The 2026-10-03 review found panic-safety gaps, repeated statistics analysis during bulk loads and misleading benchmark labels, and proposed ten follow-up changes, all shipped in 0.3.0.

### Resource Safety

Caught panics could leave a transaction open or lose a checked-out reader. The follow-up patch restored transaction, dictionary and pooled-reader cleanup.

Panic payloads and speculative burned ids are retained. Regression tests are under [[tests#Recovery]]. Bounded borrowing followed in `add-query-budgets`: reader timeouts, deadlines, cancellation and result limits ([[query#Query Budgets]]).

### Performance Evidence

The review exercised [[roadmap#Benchmarks]] with churn, cascade size, query size, path bounds, pool sizes, cache sizes and statistics thresholds, as exploratory local runs.

Point SPARQL queries stayed near 58 microseconds from 10,000 to 100,000 statements. As-of lookups rose about 30 times with 1,000 versions per key. Repeated full analysis added bulk-load cost, and reduced layered triangle fixtures supported the [[query#Physical Planning#LFTJ]] operator.

### Proposed Priorities

Resource safety and ingestion observability came first, then text recall and an agent adapter. All ten proposed changes were implemented and archived on 2026-10-04 ([[roadmap#Milestones]]).
