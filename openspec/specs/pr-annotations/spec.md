# pr-annotations Specification

## Purpose
TBD - created by archiving change add-pr-annotations. Update Purpose after archive.
## Requirements
### Requirement: PR context resolution
The CLI SHALL resolve PR-comment context (repository, PR number, token) from CI environment variables: `GITHUB_REPOSITORY` for the repo, `GITHUB_TOKEN` for auth, and the PR number from the `pull_request` object in the file at `GITHUB_EVENT_PATH` when `GITHUB_EVENT_NAME` is `pull_request`. When any of these is missing or unreadable, PR-comment context SHALL be absent and no HTTP call SHALL be attempted.

#### Scenario: Full GitHub Actions pull_request context present
- **WHEN** `GITHUB_REPOSITORY`, `GITHUB_TOKEN`, `GITHUB_EVENT_NAME=pull_request` and a readable `GITHUB_EVENT_PATH` with a `pull_request.number` are all set
- **THEN** PR context resolves with that repository, PR number, and token

#### Scenario: Push event, not a pull request
- **WHEN** `GITHUB_EVENT_NAME` is `push` (or unset)
- **THEN** PR context is absent

#### Scenario: Missing token
- **WHEN** `pull_request` context is otherwise present but `GITHUB_TOKEN` is unset
- **THEN** PR context is absent

### Requirement: Marker-based comment upsert
Given resolved PR context, a marker string, and a Markdown body, the CLI SHALL list existing issue comments on the PR across **all pages** (GitHub's issue-comments endpoint paginates; the CLI SHALL request `per_page=100` and follow `Link: rel="next"` headers until exhausted), find the first whose body contains the marker, and `PATCH` it with the new body if found, or `POST` a new comment (body prefixed with the marker as an HTML comment) if not found. Re-running with the same marker SHALL update the same comment rather than creating a new one, regardless of how many other comments exist on the PR.

#### Scenario: First run creates a comment
- **WHEN** no existing comment contains the marker
- **THEN** a new comment is created containing the marker and the given body

#### Scenario: Later run updates the same comment
- **WHEN** an existing comment already contains the marker
- **THEN** that comment is patched in place and no new comment is created

#### Scenario: Marker comment beyond the first page
- **WHEN** the PR has more than 100 comments and the one containing the marker is on a later page
- **THEN** the upsert still finds it via pagination and patches it in place instead of creating a duplicate

### Requirement: Fail-safe comment posting
A `--pr-comment` failure (absent context, network error, non-2xx response, timeout) SHALL NOT change the command's exit code or suppress its normal output. Errors SHALL be logged as a warning to stderr. Every request to the GitHub API SHALL have a bounded timeout and a single attempt — no retries — so that a hung or slow GitHub API response cannot hang the CI step, mirroring the same requirement already made of telemetry uploads.

#### Scenario: GitHub API unreachable
- **WHEN** PR context resolves but the GitHub API request fails
- **THEN** the command completes with its normal exit code and a warning is printed to stderr

#### Scenario: GitHub API hangs
- **WHEN** PR context resolves but the GitHub API does not respond within the configured timeout
- **THEN** the request is aborted at the timeout, the command completes with its normal exit code, and a warning is printed to stderr

#### Scenario: Not running in a pull request
- **WHEN** PR context is absent
- **THEN** the command completes normally without attempting any HTTP call and without printing a warning

### Requirement: Affected-scope comment
`lightning affected --pr-comment` SHALL upsert a comment, marker `<!-- lightning:affected -->`, summarizing the selection outcome: the affected module count out of total when a concrete selection was made, or the everything-affected reason when selection degraded.

#### Scenario: Concrete selection
- **WHEN** `lightning affected --pr-comment` runs and selection yields a concrete module list
- **THEN** the upserted comment states the affected module count out of the total and lists the modules

#### Scenario: Everything-affected degradation
- **WHEN** selection degrades to everything-affected
- **THEN** the upserted comment states that all modules are considered affected and includes the reason

### Requirement: Flaky-tests comment
`lightning upload --pr-comment` SHALL, after a successful upload, fetch `GET <server>/api/flaky`, filter to tests present in the just-uploaded run's results with `score > 0`, and upsert a comment (marker `<!-- lightning:flaky -->`) listing each with its score and a link to its test page. When no such tests are found, the comment SHALL say so explicitly rather than being skipped.

#### Scenario: Flaky tests present in this run
- **WHEN** the upload includes one or more tests that also appear in `/api/flaky` with `score > 0`
- **THEN** the upserted comment lists each with its score and a link to `<server>/tests/<id>`

#### Scenario: No flaky tests in this run
- **WHEN** none of the uploaded run's tests appear in `/api/flaky` with `score > 0`
- **THEN** the upserted comment explicitly states that no flaky tests were observed in this run

