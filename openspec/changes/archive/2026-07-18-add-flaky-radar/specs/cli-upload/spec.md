## ADDED Requirements

### Requirement: Parse JUnit XML reports by glob
`lightning upload` SHALL collect JUnit XML report files matching a glob pattern relative to the working directory, defaulting to `**/build/test-results/**/*.xml`, overridable via `--glob`. For each `<testcase>` it SHALL extract class name, test name, duration, and status (`pass`, `fail`, `error`, `skip`) from `failure`/`error`/`skipped` child elements.

#### Scenario: Default glob finds Gradle reports
- **WHEN** `lightning upload` runs in a directory containing `app/build/test-results/test/TEST-com.example.FooTest.xml`
- **THEN** the file is parsed and its testcases are included in the upload payload

#### Scenario: No reports found
- **WHEN** no files match the glob
- **THEN** the command exits with a non-zero status and an error message naming the glob

#### Scenario: Failure status extraction
- **WHEN** a `<testcase>` contains a `<failure>` child
- **THEN** the result status is `fail` and the failure message is captured

### Requirement: Git and CI metadata extraction
The upload SHALL include commit SHA and branch, taken from CI environment variables when present (`GITHUB_SHA`, `GITHUB_REF_NAME` at minimum) and otherwise from the local git repository. It SHALL include a CI run URL when derivable from GitHub Actions environment variables. `--sha` and `--branch` flags SHALL override both sources.

#### Scenario: GitHub Actions environment
- **WHEN** `GITHUB_SHA` and `GITHUB_REF_NAME` are set
- **THEN** the payload uses their values without invoking git

#### Scenario: Local git fallback
- **WHEN** no CI environment variables are set and the working directory is a git repository
- **THEN** SHA and branch are read via git

### Requirement: Idempotent upload
The CLI SHALL compute a stable `run_key` for the upload: the `--run-key` flag if given; else a key derived from GitHub Actions run identity (`GITHUB_REPOSITORY`, `GITHUB_RUN_ID`, `GITHUB_RUN_ATTEMPT`); else a hash of the SHA, branch, and parsed results. It SHALL POST the payload to `<server>/api/runs` where `<server>` comes from `--server` or `LIGHTNING_SERVER`.

#### Scenario: Repeated upload of the same run
- **WHEN** `lightning upload` is executed twice with identical inputs and environment
- **THEN** both invocations send the same `run_key` and the server stores the run only once

#### Scenario: Server unreachable
- **WHEN** the server cannot be reached
- **THEN** the command exits non-zero with a clear error
