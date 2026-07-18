# server-ingest Specification

## Purpose
TBD - created by archiving change add-flaky-radar. Update Purpose after archive.
## Requirements
### Requirement: Run ingest endpoint
The server SHALL accept `POST /api/runs` with a JSON payload containing `run_key`, `sha`, `branch`, optional `ci_url`, and a list of results (`class_name`, `name`, `status`, `time_ms`, optional `message`). It SHALL persist the run and its results in SQLite within one transaction and respond with the run id.

#### Scenario: Successful ingest
- **WHEN** a valid payload with a new `run_key` is posted
- **THEN** the server stores one run row and one result row per execution and returns `201` with the run id

#### Scenario: Malformed payload
- **WHEN** the JSON body is invalid or missing required fields
- **THEN** the server responds with `400` and stores nothing

### Requirement: Deduplication by run_key
`run_key` SHALL be unique. Posting a payload whose `run_key` already exists SHALL write nothing and return the existing run id with a deduplication indicator.

#### Scenario: Duplicate run_key
- **WHEN** the same payload is posted twice
- **THEN** the second response indicates deduplication and the stored data is unchanged

### Requirement: Test identity normalization on ingest
The server SHALL map results to test rows keyed by `(class_name, name)` after normalization: trimmed, internal whitespace collapsed, volatile tokens (hex addresses, UUIDs) replaced with `_`. The same normalized identity across runs SHALL resolve to the same test row.

#### Scenario: Same test across runs
- **WHEN** two runs report a result for the same normalized `(class_name, name)`
- **THEN** both results reference a single test row

### Requirement: Retention
The server SHALL delete runs and builds older than a configurable number of days (default 90, via `--retention-days` or `LIGHTNING_RETENTION_DAYS`), together with their results and task executions and any tests left without results. Pruning SHALL happen at startup and after each successful ingest.

#### Scenario: Old run pruned
- **WHEN** a run's creation time is older than the retention cutoff and an ingest completes
- **THEN** the run and its results are no longer stored

#### Scenario: Old build pruned
- **WHEN** a build's creation time is older than the retention cutoff and an ingest completes
- **THEN** the build and its task executions are no longer stored

### Requirement: Build ingest endpoint
The server SHALL accept `POST /api/builds` with a JSON payload containing `build_key`, optional `sha`, `branch`, `ci_url`, `gradle_version`, `java_version`, plus `outcome` (`success` or `failed`), `requested_tasks`, `total_ms`, `configuration_ms`, and a list of tasks (`path`, `outcome` in `success`/`up-to-date`/`from-cache`/`failed`/`skipped`, `duration_ms`). It SHALL persist the build and its task executions in SQLite within one transaction and respond `201` with the build id. Malformed payloads (invalid JSON, missing required fields, unknown outcome values) SHALL get `400` and store nothing.

#### Scenario: Successful build ingest
- **WHEN** a valid payload with a new `build_key` is posted
- **THEN** the server stores one build row and one task execution row per task and returns `201` with the build id

#### Scenario: Malformed build payload
- **WHEN** the JSON body is invalid, misses required fields, or contains an unknown outcome
- **THEN** the server responds with `400` and stores nothing

### Requirement: Deduplication by build_key
`build_key` SHALL be unique. Posting a payload whose `build_key` already exists SHALL write nothing and return the existing build id with a deduplication indicator.

#### Scenario: Duplicate build_key
- **WHEN** the same build payload is posted twice
- **THEN** the second response indicates deduplication and the stored data is unchanged

### Requirement: Builds list API
The server SHALL serve `GET /api/builds` returning recent builds as JSON, newest first, including id, `build_key`, `sha`, `branch`, `outcome`, `total_ms`, `configuration_ms`, task counts per outcome, and creation time.

#### Scenario: Recent builds listed
- **WHEN** builds exist and `GET /api/builds` is fetched
- **THEN** the response lists them newest first with their timings and outcome counts

