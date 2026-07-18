## Context

Phase 2 of the lightning platform (`docs/design/lightning-platform.md`): build telemetry via an init script, published to the existing single-binary server. Phase 1 established the patterns this change follows: rusqlite behind a `Mutex`, idempotent ingest keyed by a client-computed key, retention pruning, server-rendered HTML, JSON API next to the pages.

## Goals / Non-Goals

**Goals:**
- Init script embedded in the CLI, extracted with one command, attached with `--init-script`.
- Per-task timings/outcomes, configuration vs total time, requested tasks, Gradle/JDK versions, git/CI metadata.
- Idempotent `POST /api/builds`; builds list, build detail, per-branch trend screens.
- Telemetry is strictly fail-safe: it must never fail or slow down a build materially.

**Non-Goals:**
- Remote build cache and cache analytics (phase 3).
- Module graph extraction (phase 4 `sync` — a separate init script concern).
- Deep traces (test-level events, dependency resolution, plugin application breakdown).
- Gradle plugin published to a repository; the init script is the only delivery vehicle.

## Decisions

### D1. Init script DSL: Groovy

The script is written in the Groovy DSL (`lightning.init.gradle`), not Kotlin (`.init.gradle.kts`). Reasons: Groovy init scripts are supported and behave identically across every Gradle version we care about (6.1+ through 9.x), compile faster at startup (no Kotlin script compilation on first run), and avoid the Kotlin DSL's historically weaker init-script support. The script is a single self-contained file with no external dependencies — everything it needs is in the Gradle API and the JDK.

### D2. Collection mechanism: BuildService + OperationCompletionListener

A `Plugin<Gradle>` class defined inside the init script is applied to the `Gradle` object. It obtains `BuildEventsListenerRegistry` via `@Inject` service injection (public API for plugins since Gradle 6.1) and registers a shared `BuildService` implementing `OperationCompletionListener` and `AutoCloseable`:

- `onFinish(FinishEvent)` records every `TaskFinishEvent`: task path, start/end clock times, and outcome derived from the result type (`TaskSuccessResult.isUpToDate()` → `up-to-date`, `isFromCache()` → `from-cache`, plain success → `success`; `TaskFailureResult` → `failed`; `TaskSkippedResult` → `skipped`).
- `close()` runs when Gradle disposes services at build finish — after all tasks — and is where the payload is assembled and POSTed.

Rejected alternatives: `gradle.buildFinished`/`BuildListener` (deprecated, removed in Gradle 9, configuration-cache incompatible); `gradle.taskGraph.afterTask` (same fate); Develocity-style internal `BuildOperationListener` (internal API, breaks across versions). The chosen pair is the documented, configuration-cache-compatible replacement.

### D3. Timing model

The script records `buildStartMs = System.currentTimeMillis()` when it is evaluated (init scripts run before settings/projects are configured) and passes it as a service parameter. In the service:

- `total_ms` = close time − `buildStartMs`.
- `configuration_ms` = earliest task start − `buildStartMs` (0 when no tasks ran).
- per-task `duration_ms` = task end − task start (wall clock; parallel tasks overlap, so the sum can exceed `total_ms` — the UI never sums them into a total).

This excludes JVM/daemon startup (invisible to any in-build mechanism) and counts init+settings+configuration as "configuration". Known limitation: on a configuration-cache hit the init script is not re-evaluated, so the cached `buildStartMs` makes both derived values wrong for that build; listeners still fire and task data stays correct. Accepted for phase 2 — the typical target (ephemeral CI runners) rarely hits the configuration cache, and per-task data is the primary signal.

### D4. Build outcome

`outcome` = `failed` if any task failed, else `success`. `gradle.buildFinished` would give the authoritative result but is removed in Gradle 9; the Flow API (`FlowProviders`) exists only since 8.1 and is awkward from an init script. If configuration fails before any task runs, the service is never instantiated and nothing is posted — an unreported build, accepted (there is nothing useful to chart about it yet).

### D5. Event schema (one JSON document per build)

```json
{
  "build_key": "1f0c…-uuid",
  "sha": "abc123…" | null,
  "branch": "main" | null,
  "ci_url": "https://github.com/o/r/actions/runs/1" | null,
  "outcome": "success" | "failed",
  "requested_tasks": "build check",
  "gradle_version": "9.6.1",
  "java_version": "21.0.11",
  "total_ms": 12345,
  "configuration_ms": 2345,
  "tasks": [
    { "path": ":app:compileJava", "outcome": "success", "duration_ms": 812 },
    { "path": ":app:test", "outcome": "from-cache", "duration_ms": 3 }
  ]
}
```

Task outcomes: `success`, `up-to-date`, `from-cache`, `failed`, `skipped` (server validates; unknown values are rejected as malformed). `requested_tasks` is the space-joined `startParameter.taskNames` (may be empty for default tasks). `sha`/`branch` come from GitHub Actions env (`GITHUB_SHA`, `GITHUB_HEAD_REF`/`GITHUB_REF_NAME`) or `git rev-parse` in the root project dir, `null` when neither works; `ci_url` mirrors phase 1's GitHub Actions derivation. Metadata is resolved in `close()` (execution time) so env values are never baked into configuration-cache state.

### D6. build_key: random UUID per build invocation

`build_key` = `UUID.randomUUID()` generated when the service first assembles the payload. Unlike phase-1 runs (one upload per CI job, so CI identity works as a key), one CI job commonly runs several Gradle invocations, and two builds with identical inputs are genuinely two builds — so a content- or CI-derived key would wrongly merge them. A per-invocation UUID makes every build distinct while keeping the server-side `UNIQUE(build_key)` contract: the script POSTs once, and any replay of the same payload (curl retry, proxy replay, manual re-send) dedupes exactly like `run_key`.

### D7. Fail-safe policy

Telemetry must never break or block a build:

- The entire plugin application is wrapped in try/catch; registration failure logs one warning and disables telemetry.
- `close()` wraps payload assembly and POST in try/catch; any exception is logged as a single warning line (prefixed `lightning:`) and swallowed.
- HTTP uses `HttpURLConnection` with 5 s connect / 10 s read timeouts, one attempt, no retries (retries would delay build exit).
- `git` subprocesses get a 2 s timeout; failure means `null` metadata, not an error.
- No server URL configured → the script does nothing except log one info line.

### D8. Server URL configuration

URL resolution order: Gradle property `lightning.url` (`-Plightning.url=…` or `gradle.properties`), else env `LIGHTNING_URL`. The property is read from `gradle.startParameter.projectProperties` at registration and passed as a service parameter; the env var is read in `close()`. The script POSTs to `<url>/api/builds`.

### D9. CLI command

`lightning init-script [--out <path>]` writes the embedded script (`include_str!` of `crates/cli/assets/lightning.init.gradle`) to `--out`, or to stdout when omitted. Overwrites an existing file (the script is versioned with the binary; CI regenerates it every run). CI usage:

```sh
lightning init-script --out lightning.init.gradle
./gradlew build --init-script lightning.init.gradle
```

### D10. Storage schema

```sql
builds(id INTEGER PRIMARY KEY, build_key TEXT UNIQUE NOT NULL, sha TEXT, branch TEXT,
       ci_url TEXT, outcome TEXT NOT NULL, requested_tasks TEXT NOT NULL,
       gradle_version TEXT, java_version TEXT,
       total_ms INTEGER NOT NULL, configuration_ms INTEGER NOT NULL,
       created_at TEXT NOT NULL DEFAULT (datetime('now')))
task_executions(id INTEGER PRIMARY KEY,
       build_id INTEGER NOT NULL REFERENCES builds(id) ON DELETE CASCADE,
       path TEXT NOT NULL, outcome TEXT NOT NULL, duration_ms INTEGER NOT NULL)
CREATE INDEX idx_task_executions_build ON task_executions(build_id)
```

Ingest mirrors phase 1: one transaction, dedup check on `build_key` first, 201/200 + `{build_id, deduplicated}`. Retention: `prune()` also deletes builds older than the same `retention_days` window (task executions cascade). Tables are independent of the phase-1 run tables; correlating a build with a test run is a later concern.

### D11. Cache avoidance metric

The builds list shows an "avoided" ratio: `(up-to-date + from-cache) / total tasks` (skipped tasks excluded from the numerator, included in the denominator). This is deliberately the broad avoidance number, not the strict remote-cache hit rate — phase 3 owns cache analytics. 0 tasks → "—".

### D12. UI

- `/builds`: table of recent builds (newest first, capped at 100): time, branch, short SHA, outcome, requested tasks, total duration, avoided ratio; row links to detail. CI link when present.
- `/builds/{id}`: metadata line, configuration vs execution split, task outcome breakdown (counts per outcome), slowest tasks table (top 20 by duration, with outcome).
- `/trends`: per branch over its recent builds (last 50 per branch): build count, median total duration, median avoided ratio, inline duration bar scaled to the slowest branch median. Median over successful builds only (failed builds stop early and would skew durations down).
- Navigation: the shared page shell gains a `flaky · builds · trends` nav; `/` remains the flaky list.

Durations render as `1m 23s` / `840ms` via a shared formatter.

## Risks / Trade-offs

- [Internal-free API surface still varies across Gradle majors] → only `BuildService`, `OperationCompletionListener`, `BuildEventsListenerRegistry` injection, and the tooling-events result types are used; all are stable public API since 6.1 and verified against 9.x in E2E.
- [POST at build finish adds latency] → bounded by timeouts (≤15 s worst case, ~ms typical on LAN); one attempt only.
- [Configuration-cache hit skews configuration/total times] → accepted, documented in D3; task-level data unaffected.
- [UUID build_key means an interrupted POST that is retried by a proxy could still dedupe, but a re-run build never merges] → exactly the intended semantics.
- [Payload size on huge builds (10k tasks)] → ~100 bytes/task ≈ 1 MB JSON; fine for axum/SQLite; no pagination needed at ingest.

## Migration Plan

Additive: new tables are created by the existing `CREATE TABLE IF NOT EXISTS` startup path; existing phase-1 data and endpoints are untouched.

## Open Questions

None blocking. Deferred: authoritative build outcome via Flow API (when minimum Gradle allows), correlating builds with test runs, per-machine dimension for trends.
