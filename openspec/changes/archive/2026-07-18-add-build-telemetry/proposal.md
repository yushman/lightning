## Why

Phase 1 shows which tests flake; it says nothing about why CI is slow. Phase 2 of the lightning platform ("build scans lite") adds build telemetry: per-task timings, outcomes, cache avoidance, configuration vs execution time — collected by a Gradle init script and browsable in the same server UI. This is the first, deliberately small step inside the build: one `--init-script` flag, never a plugin in the build itself.

## What Changes

- `lightning init-script`: new CLI command that emits a Groovy init script embedded in the `lightning` binary (to a file via `--out`, or stdout). The script registers a `BuildService` + `OperationCompletionListener` (Gradle public build-event API), collects per-task timings and outcomes (SUCCESS / UP-TO-DATE / FROM-CACHE / FAILED / SKIPPED), configuration and total build time, requested tasks, Gradle/JDK versions, git SHA/branch and CI metadata, and POSTs one JSON document to the lightning server when the build finishes. Telemetry is fail-safe: any error is swallowed and logged; the build never breaks.
- Server: `POST /api/builds` ingest, idempotent via `build_key` (analogous to `run_key`), new SQLite tables `builds` and `task_executions`, retention pruning extended to builds. `GET /api/builds` JSON list.
- Web UI (same server-rendered style): `/builds` list (time, branch, outcome, duration, cache avoidance), `/builds/{id}` detail (configuration vs execution split, outcome breakdown, slowest tasks), `/trends` median build duration per branch. Shared navigation so flaky radar and builds are both reachable.

## Capabilities

### New Capabilities

- `build-telemetry`: init script embedded in the CLI, its extraction command, the collected event schema, and the fail-safe publishing contract.

### Modified Capabilities

- `server-ingest`: add build ingest endpoint with `build_key` dedup; retention extended to builds.
- `web-ui`: add builds list, build detail, and branch trend screens; add cross-section navigation.

## Impact

- CLI: new `init-script` subcommand; the Groovy script ships inside the binary (`include_str!`).
- Server: new tables, `POST /api/builds`, `GET /api/builds`, three new HTML pages, nav added to existing pages.
- No new Rust dependencies. No changes to phase-1 flaky behavior.
- CI integration grows by one flag: `gradle build --init-script lightning.init.gradle` after `lightning init-script --out lightning.init.gradle`.
