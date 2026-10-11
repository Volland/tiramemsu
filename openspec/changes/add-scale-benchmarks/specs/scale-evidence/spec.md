## ADDED Requirements

### Requirement: Declared scale ceiling
The project SHALL declare its tested scale ceiling as 10⁷ statements per file, 1 000 versions per hot key and 100 single-statement write transactions per second sustained for 60 seconds, on a laptop-class machine with WAL journaling and `synchronous = NORMAL`. The ceiling SHALL be documented as tested behavior and SHALL NOT be enforced as a limit by the engine.

#### Scenario: Beyond the ceiling still works
- **WHEN** a file holds more than 10⁷ statements and another statement is asserted
- **THEN** the assertion succeeds as it would below the ceiling

#### Scenario: Sustained write rate
- **WHEN** the scale harness runs single-statement `assert` transactions for 60 seconds against a file of 10⁷ statements
- **THEN** it records at least 100 commits per second with a p99 commit latency of at most 50 ms, or the gate fails

### Requirement: Scale harness with checked results
A scale harness SHALL build deterministic fixtures at 10⁶ and 10⁷ statements from a seed and measure point and 2-hop lookups under the now, as-of and valid-at views, churn at 1, 10, 100 and 1 000 versions per key, history depth (`history` view and `events_since` over deep keys), path shapes, supersede-cascade cost, bulk-import throughput and sustained writes. Each measurement SHALL compare its row count with the fixture's known answer before timing and SHALL fail instead of recording a time on a mismatch. Results SHALL be written as JSON with the metric name, unit, sample count, p50, p95, p99, row count and the captured query plan.

#### Scenario: Wrong result fails the run
- **WHEN** a shape returns a different number of rows than the fixture specifies
- **THEN** the harness exits with an error naming the shape, and no timing for it is written

#### Scenario: Deterministic fixture
- **WHEN** the fixture is generated twice from the same seed and size
- **THEN** both files hold the same statements with the same eids

### Requirement: History-depth targets
The harness SHALL report, per view, the throughput ratio of lookups on keys with N versions (N = 10, 100, 1 000) to lookups on keys with one version. The as-of ratio target SHALL be at least 0.70 and the current-view ratio target at least 0.90 at 1 000 versions. Until the targets are met they SHALL be recorded and reported as missed; once met they SHALL be gating.

#### Scenario: Ratio reported
- **WHEN** the harness runs the churn shapes
- **THEN** its output contains the as-of and current-view ratios for 10, 100 and 1 000 versions, each with the target and whether it is met

### Requirement: Nightly benchmark gate
A scheduled CI workflow SHALL run the harness nightly at 10⁶ statements, three times, and compare the median p50 of each gated metric with a checked-in baseline for the same runner class. The gate SHALL fail when a gated metric is more than 20 % slower than its baseline, when a gating ratio target is missed, or when the sustained-write requirement is not met. A weekly run at 10⁷ statements SHALL record ceiling evidence. Publishing workflows SHALL refuse to release while the latest nightly gate is failing. The baseline SHALL change only by an explicit commit.

#### Scenario: Regression blocks the gate
- **WHEN** a change makes the as-of point lookup p50 30 % slower than the baseline
- **THEN** the nightly gate fails and names the metric, its baseline and its new value

#### Scenario: Release refused on a red gate
- **WHEN** a release workflow starts while the latest nightly gate failed
- **THEN** the release job stops before publishing and points to the failing run

### Requirement: oxilite head-to-head is published
oxilite's `write-cost` and `as-of-latency` harnesses SHALL be ported to run on both engines over the same data with equal result counts, run at 80 000 to 400 000 triples, and their results SHALL be published with the machine description whatever they show.

#### Scenario: Equal answers before comparing
- **WHEN** the as-of-latency port runs a past-state query on both engines
- **THEN** both return the same number of rows before any time is recorded
