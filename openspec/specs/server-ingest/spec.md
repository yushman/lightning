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
The server SHALL delete runs older than a configurable number of days (default 90, via `--retention-days` or `LIGHTNING_RETENTION_DAYS`), together with their results and any tests left without results. Pruning SHALL happen at startup and after each successful ingest.

#### Scenario: Old run pruned
- **WHEN** a run's creation time is older than the retention cutoff and an ingest completes
- **THEN** the run and its results are no longer stored

