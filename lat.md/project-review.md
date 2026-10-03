# Project Review

The 2026-10-03 review records measured behavior and proposed work without changing the implemented architecture. The full report and reproducible probes are in `docs/project-review.md` and `docs/review-results/`.

## Resource Safety

Caught panics can leave a transaction open or lose a checked-out reader. Review transaction and reader lifetimes before expanding agent integrations.

The original probes reproduced an unusable writer and reader loss after caught panics. The follow-up patch restores transaction, dictionary, and pooled-reader cleanup while retaining panic payloads and speculative burned ids. Regression tests are under [[tests#Recovery]]. Bounded borrowing followed in `add-query-budgets`: reader timeouts, deadlines, cancellation and result limits ([[query#Query Budgets]]).

## Performance Evidence

The review exercises [[roadmap#Benchmarks]] with churn, cascade size, query size, path bounds, pool sizes, cache sizes, and statistics thresholds. The measurements are exploratory local runs.

Point SPARQL queries remain near 58 microseconds from 10,000 to 100,000 statements. As-of lookups rise substantially with 1,000 versions per key. Repeated full analysis adds bulk-load cost, and reduced layered triangle fixtures support the deferred [[query#Physical Planning#LFTJ]] operator.

## Proposed Priorities

Resource safety and ingestion observability come first, followed by text recall and an agent adapter. All ten proposed changes were implemented and archived on 2026-10-04 ([[roadmap#Milestones]]).

The report relates retrieval to existing roadmap work, suggests saved-answer invalidation with explicit provenance limitations, and recommends using the existing path and native-operator boundaries for subsequent extensions.
