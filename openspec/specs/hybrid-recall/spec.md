# hybrid-recall Specification

## Purpose
Hybrid recall: text and vector recall in one view and snapshot, fused by reciprocal rank (k = 60), ranked by evidence on ties, and optionally expanded to each hit's layers and subject facts. One recall serves Rust, SPARQL, Cypher, the JSON bridge, MCP and the bindings. Archived from `openspec/changes/archive/2026-10-11-add-hybrid-recall`.

## Requirements

### Requirement: Fused recall over text and vectors
`View::recall` SHALL accept a text query, a vector query or both, SHALL run each requested channel as its standalone recall in one snapshot with the candidate depth (default `max(4 × limit, 50)`) as its limit, and SHALL fuse the results by reciprocal rank: a statement's score is the sum over the channels that returned it of `1 / (rrf_k + rank)`, with `rrf_k` defaulting to 60. It SHALL return at most `limit` (default 10) fused hits. A request with neither channel SHALL fail with `InvalidQuery`.

#### Scenario: Agreement ranks first
- **WHEN** statement A is ranked 2 by text recall and 3 by vector recall, and statement B is ranked 1 by text recall only
- **THEN** A's score is `1/62 + 1/63`, B's is `1/61`, and A is ranked above B

#### Scenario: Single channel
- **WHEN** only a vector query is given
- **THEN** the hits are the vector recall's hits in the same order, each with its score computed from its vector rank

#### Scenario: Neither channel
- **WHEN** a recall request names neither text nor vector
- **THEN** it fails with `InvalidQuery`

### Requirement: No invented scores
Each fused hit SHALL report, per channel, the channel's own score (`lexical` or `similarity`) and rank (`text_rank` or `vector_rank`) when that channel returned it, and `None` otherwise. The only derived number SHALL be the RRF score. Evidence components SHALL be reported as read, with absent layers reported as absent.

#### Scenario: Absent channel
- **WHEN** a hit was found by vector recall but not by text recall
- **THEN** its `lexical` and `text_rank` are `None`, not zero or a placeholder

#### Scenario: Reproducible fusion
- **WHEN** a caller recomputes each fused hit's score from the reported channel ranks and `rrf_k`
- **THEN** the recomputed values equal the reported ones

### Requirement: Evidence ordering
Fused hits SHALL be ordered by the policy `tiramemsu-recall-rank/1`: RRF score descending, then confidence descending with absent last, confirmation count descending, distinct authors descending, `added_at` newer first, and statement id ascending. Evidence SHALL be read in the query's view. Each returned statement SHALL carry its 1-based rank in the returned list.

#### Scenario: Evidence breaks a tie
- **WHEN** two hits have equal scores and one has confidence 0.9 while the other has none
- **THEN** the hit with confidence 0.9 is ranked first

#### Scenario: Deterministic
- **WHEN** the same recall runs twice on an unchanged database
- **THEN** both runs return the same statements in the same order

### Requirement: Bounded expansion
When `expand` is `n > 0`, recall SHALL add for each fused hit, in rank order, up to `n` statements visible in the view whose subject is the hit's statement (its layers) and then up to `n` visible statements whose subject is the hit's subject node, other than the hit. Expanded statements SHALL carry `via` set to the hit they were reached from, SHALL have score 0 and no channel scores or ranks, SHALL be ranked after every fused hit, and SHALL not repeat a statement already returned. With `expand` 0, no statement SHALL have `via` set.

#### Scenario: Layers and subject facts
- **WHEN** a hit `(alice worksAt acme)` has a confidence layer and `alice` has another fact `(alice livesIn lisbon)`, and recall runs with `expand = 5`
- **THEN** the result contains the confidence layer and `(alice livesIn lisbon)`, each with `via` = the hit's eid, ranked after the fused hits

#### Scenario: Expansion cap
- **WHEN** a hit's subject has 20 other visible facts and recall runs with `expand = 3`
- **THEN** exactly 3 of them are returned for that hit

#### Scenario: Expansion respects the view
- **WHEN** a recall in an as-of view expands its hits
- **THEN** every expanded statement is visible in that view

### Requirement: Unavailable channels fail the call
If a requested channel cannot run, recall SHALL fail with that channel's typed error (`MissingCapability`, `TextIndexUnavailable`, or `InvalidQuery` for an unknown embedding model or invalid vector) and SHALL return no hits. The vector channel SHALL need no host capability and no text index.

#### Scenario: Text channel without FTS5
- **WHEN** a recall with a text query and a vector query runs on a host without FTS5
- **THEN** it fails with `MissingCapability` and returns no vector-only fusion

#### Scenario: Vector-only recall without the text index
- **WHEN** a recall with only a vector query runs on a file whose text index was never built
- **THEN** it returns the vector channel's hits

#### Scenario: Unknown model
- **WHEN** a recall names an embedding model that is not defined
- **THEN** it fails with `InvalidQuery`

### Requirement: One hybrid recall from every surface
The Rust API, SPARQL `tm:recall*` patterns, the Cypher procedure `tiramemsu.recall`, the JSON bridge `recall` op, the MCP `recall` tool and the Python and Node bindings SHALL run the same logical recall, so equal requests return the same statements, scores, ranks and `via` values, and each call SHALL be one operation under the caller's query budget. SPARQL and Cypher forms SHALL follow in a later phase with the same results.

#### Scenario: Surfaces agree
- **WHEN** the same text and vector are recalled with `expand = 2` through `View::recall`, the bridge, MCP, Python and Node
- **THEN** all return the same statements, scores, ranks and `via` values

#### Scenario: Budgeted recall
- **WHEN** a recall runs under a budget whose deadline expires during expansion
- **THEN** it fails with `DeadlineExceeded` and returns no partial result
