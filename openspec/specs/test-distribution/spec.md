# test-distribution Specification

## Purpose
TBD - created by archiving change design-test-distribution. Update Purpose after archive.
## Requirements
### Requirement: Static timing-balanced shard planning (stage 1)
`lightning affected --format github-matrix --shards <n>` SHALL emit at most `n` matrix entries, each with a `shard` id and a `tasks` string of space-separated `:module:task` paths covering **all** affected modules exactly once, with task names resolved from `lightning.lock` per module and bins packed by expected duration from timing history using greedy longest-first packing. When history is unavailable for a module, its weight SHALL fall back to a fixed default rather than failing or dropping the module.

#### Scenario: Balanced bins cover every affected module
- **WHEN** 10 modules are affected and `--shards 3` is given
- **THEN** the matrix has at most 3 entries whose task paths partition the 10 modules (each exactly once), packed by recorded durations

#### Scenario: Cold start degrades to uniform weights
- **WHEN** no timing history is reachable
- **THEN** packing uses equal weights, a note is printed to stderr, and no module is omitted

### Requirement: Distribution sessions with a work-unit ledger (stage 2)
The server SHALL manage distribution sessions: a session is created with a commit SHA, run identity, required worker labels, and a set of work units `(module, task, optional class filters, weight)`. The server SHALL track every unit in a ledger from `pending` through `leased` to a terminal per-attempt state, and SHALL derive the session verdict exclusively from the ledger: `passed` (all units completed, no test failures), `failed` (all units completed, at least one test failure), or `incomplete` (any unit exhausted its attempts or the session wall-clock timeout fired). A session SHALL never report a verdict that omits a unit.

#### Scenario: Unit exhausts attempts
- **WHEN** a unit fails with infrastructure errors on `max_attempts` consecutive attempts
- **THEN** the session ends `incomplete`, naming the unit, and never `passed` or `failed`

#### Scenario: Verdict accounts for every unit
- **WHEN** a session reaches a terminal state
- **THEN** every created unit is in a terminal state and appears in the session report

### Requirement: Lease-based dispatch with heartbeats (stage 2)
Workers SHALL obtain units via long-poll lease requests carrying their labels; the server SHALL lease a unit only to a worker whose labels are a superset of the session's required labels, longest-expected-duration first. Leases SHALL carry a TTL extended by worker heartbeats; a lease whose TTL lapses SHALL be revoked and the unit requeued as a new attempt. Delivery is at-least-once: when multiple attempts of one unit report completion, the server SHALL accept the first and discard the rest.

#### Scenario: Worker dies mid-unit
- **WHEN** a worker stops heartbeating after leasing a unit
- **THEN** after TTL expiry the unit returns to the queue and a subsequent lease hands it to another worker

#### Scenario: Duplicate completion discarded
- **WHEN** two attempts of the same unit both report results
- **THEN** exactly one attempt's results enter the merged run

### Requirement: Infrastructure failures retried, test failures reported (stage 2)
A completed Gradle invocation with parseable test results SHALL be terminal for its unit regardless of test outcomes — the scheduler SHALL NOT re-execute failed tests. Only infrastructure failures (worker crash, lease expiry, checkout failure, missing results, unit timeout) SHALL requeue a unit, up to `max_attempts`; results from infrastructure-failed attempts SHALL be discarded, never merged.

#### Scenario: Failing test is a result, not a retry
- **WHEN** a unit's Gradle run exits non-zero with JUnit XML showing test failures
- **THEN** the unit is `completed`, the failures enter the merged run, and the unit is not re-executed

### Requirement: Single merged run in the flaky radar (stage 2)
When a session reaches a terminal state, the server SHALL materialize exactly one run through the existing ingest path with a run key derived from the session identity, containing the union of results from accepted attempts. Re-creating a session for the same run identity SHALL deduplicate exactly as phase-1 uploads do.

#### Scenario: Shards merge to one run
- **WHEN** a session with 8 units completes
- **THEN** the flaky radar shows one run for that CI run containing all shards' results

### Requirement: Class-level sharding never drops a test (stage 3)
When splitting a module's test task by class history, the planner SHALL assign every historically known class to exactly one include shard and SHALL create exactly one remainder shard per split module that excludes precisely the classes assigned to include shards, so any class unknown to history executes in the remainder shard by construction. Include filters SHALL be passed on the Gradle command line; exclude filters SHALL be applied only through the sanctioned init-script channel. An include filter matching no tests (deleted class) SHALL be a recognized non-fatal outcome.

#### Scenario: New class runs without history
- **WHEN** a test class absent from history exists in a split module
- **THEN** it executes in the module's remainder shard and its results appear in the merged run

### Requirement: Distribution auth reuses the shared-token pattern (stage 2)
When a distribution token is configured on the server, all distribution endpoints SHALL require it; without one they SHALL be open. Documentation SHALL state that workers execute repository code and must be trusted like CI runners — the token gates enqueueing and leasing, it is not a sandbox.

#### Scenario: Tokenless worker rejected
- **WHEN** a token is configured and a worker leases without presenting it
- **THEN** the request is rejected with an authentication error

