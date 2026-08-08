## Why

Flaky scores and affected-module scope only exist in the web UI today — a reviewer has to know lightning is running and go open it. The moment that data would actually change a decision is on the PR itself, before merge. Both signals are already computed from data lightning collects (`/api/flaky`, `affected` selection); this change only changes where they're surfaced, not how they're derived.

## What Changes

- New `crates/cli/src/github.rs`: a small GitHub REST client — resolves PR context from CI environment (`GITHUB_REPOSITORY`, `GITHUB_EVENT_NAME`, `GITHUB_EVENT_PATH`, `GITHUB_TOKEN`), and upserts a single marker-tagged issue comment (list comments, patch if a comment with the marker exists, else create).
- `lightning affected` gains `--pr-comment`: after computing selection, upserts a comment showing the affected module count and list (or the "everything" reason), marker `<!-- lightning:affected -->`.
- `lightning upload` gains `--pr-comment`: after a successful upload, queries `GET <server>/api/flaky`, filters to tests present in this run's results with `score > 0`, and upserts a comment listing them with links to the test page, marker `<!-- lightning:flaky -->`. No flaky tests in the run means the comment says so explicitly rather than staying silent (the whole point is a reviewer never has to guess whether the check ran).
- Both flags are independent and no-op silently (fail-safe, matching the telemetry init script's contract) when the required GitHub context is missing: not a `pull_request` event, no `GITHUB_TOKEN`, or the API call fails. `--pr-comment` never turns a successful `affected`/`upload` into a failing command; a GitHub API error is a warning on stderr, exit code unaffected.
- `crates/cli/Cargo.toml`: `ureq` currently builds with `default-features = false, features = ["json"]` — no TLS backend, because the server URL is self-hosted and typically plain HTTP. `api.github.com` requires HTTPS, so this adds `rustls` to ureq's feature set.

## Capabilities

### New Capabilities
- `pr-annotations`: PR context resolution from CI environment, the marker-based upsert-comment contract, and the two comment producers (affected scope, flaky tests) with their fail-safe rules.

### Modified Capabilities
- None. `affected-selection` and `flaky-scoring`'s existing requirements (module selection, `/api/flaky`) are unchanged; this change only reads their output.

## Impact

- Code: `crates/cli/src/github.rs` (new), `crates/cli/src/affected.rs`, `crates/cli/src/main.rs` (flag wiring), `crates/cli/Cargo.toml` (ureq `rustls` feature).
- External: requires `GITHUB_TOKEN` with `pull-requests: write` permission in the calling workflow — a workflow-config change on the user's side, not something lightning provisions.
- No server or DB changes: `/api/flaky` is read-only from the CLI's perspective, no new endpoint.
