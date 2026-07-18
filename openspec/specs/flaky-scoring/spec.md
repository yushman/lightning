# flaky-scoring Specification

## Purpose
TBD - created by archiving change add-flaky-radar. Update Purpose after archive.
## Requirements
### Requirement: Run-level verdict per test
For each test in a run the system SHALL derive one verdict from its executions: `pass` if all executions pass, `fail` if all fail or error, `mixed` if both passing and failing executions are present, and no verdict if only skips are present.

#### Scenario: In-run retry flip
- **WHEN** a run contains one failing and one passing execution of the same test
- **THEN** the test's verdict for that run is `mixed`

### Requirement: Flaky definition and score
Over a window of the last 50 verdicts of a test the system SHALL compute `flip_shas` (distinct SHAs with both passing and failing evidence, where `mixed` counts alone) and `flips` (adjacent pass/fail transitions in the verdict sequence, `mixed` excluded). A test SHALL be classified flaky iff `flip_shas >= 1` or `flips >= 2`, with score `round(100 * min(1, 0.6 * min(flip_shas, 3) / 3 + 0.4 * flips / max(n - 1, 1)))`, and score 0 otherwise.

#### Scenario: Same-SHA flip across two uploads
- **WHEN** a test fails in one run and passes in another run with the same SHA
- **THEN** the test is flaky with a score of at least 20

#### Scenario: Honest regression is not flaky
- **WHEN** a test's window is a sequence of passes followed only by fails, all on distinct SHAs
- **THEN** the test is not flaky and its score is 0

#### Scenario: Repeated cross-SHA flips
- **WHEN** a test's verdicts alternate pass/fail across three or more distinct SHAs
- **THEN** the test is flaky with a positive score

### Requirement: Flaky list API
The server SHALL expose `GET /api/flaky` returning flaky tests (score > 0) as JSON ordered by score descending, each with identity, score, flip counts, and last-seen information.

#### Scenario: Flaky test appears in API
- **WHEN** a test satisfies the flaky definition
- **THEN** `GET /api/flaky` includes it with its computed score

