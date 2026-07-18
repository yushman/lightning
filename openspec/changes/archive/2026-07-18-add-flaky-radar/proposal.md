## Why

Flaky tests are the most acute unsolved pain in Gradle/Android CI, and there is no OSS tooling for them (only Develocity). Phase 1 of the lightning platform ships a flaky-test radar with zero build integration: one CLI line at the end of a CI job.

## What Changes

- New Cargo workspace at repo root with two crates: `crates/cli` (binary `lightning`) and `crates/server` (binary `lightning-server`), laid out for future phases.
- `lightning upload`: parses JUnit XML reports by glob (default `**/build/test-results/**/*.xml`), extracts git SHA/branch from the repo or environment, collects CI metadata from environment variables (GitHub Actions at minimum), and POSTs a run payload to the server. Re-uploading the same run is idempotent.
- `lightning-server`: single binary with SQLite storage, configured via flags/env. Provides an ingest API, computes flaky scores, and serves a server-rendered HTML UI (no JS build chain) with three screens: flaky tests list with trends, single test history, and run summary.
- Data model decisions (test identity across runs, formal flaky definition and score, retention policy) are fixed in this change's design.md.

## Capabilities

### New Capabilities
- `cli-upload`: parse JUnit XML test reports, enrich with git and CI metadata, upload a run to the server idempotently.
- `server-ingest`: HTTP API accepting run payloads, deduplicating repeated uploads, persisting results in SQLite.
- `flaky-scoring`: test identity model, formal flaky definition, flaky score computation, and retention policy.
- `web-ui`: server-rendered HTML screens for flaky list, test history, and run summary.

### Modified Capabilities

None (no existing specs).

## Impact

- New code: entire Cargo workspace (`Cargo.toml`, `crates/cli`, `crates/server`).
- New HTTP API surface: `POST /api/runs`, JSON read endpoints, HTML pages.
- Dependencies: clap, serde, quick-xml (or similar), reqwest/ureq, axum (or similar), rusqlite.
- No build integration: CI adds a single `lightning upload` step.
