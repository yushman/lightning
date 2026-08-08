## Why

`/trends` (`web.rs::trends_page`) already reports median *build* duration per branch, but a single task regressing (a new slow dependency, a lost cache hit, a changed test suite) is invisible there until it drags the whole build's median with it — and by the time that's visible, it's been slow for a while. `task_executions` already stores every task's path, outcome, and duration per build; nothing new needs to be collected, only compared.

## What Changes

- `crates/server/src/db.rs`: new query comparing, per task path and branch, the median duration over a recent window of builds against the median over the baseline window immediately preceding it (both counting only `success`/`failed` executions — the same durations `never_cached_tasks` already treats as "real work", excluding `up-to-date`/`from-cache`/`skipped`, which are near-zero and not comparable work).
- A task is flagged regressed when `recent_median > baseline_median * (1 + threshold_pct)` AND `recent_median - baseline_median >= min_delta_ms` (an absolute noise floor so a task that goes from 20ms to 30ms — a 50% swing that's still nothing — doesn't get flagged). Deterministic and recomputable by hand, no ML — same posture as flaky scoring.
- New `GET /api/trends/regressions?branch=<name>` returning the regressed task list as JSON.
- `/trends` page gains a "Regressed tasks" section per branch beneath the existing build-duration table, listing task path, baseline median, recent median, and percent delta.
- New capability `task-trend-regressions`. No existing capability's requirements change — `build-telemetry`'s collection contract is untouched; this only reads what it already stores.

## Capabilities

### New Capabilities
- `task-trend-regressions`: the baseline/recent window comparison, the regression threshold rule, the `/api/trends/regressions` endpoint, and the `/trends` page section.

### Modified Capabilities
- None.

## Impact

- Code: `crates/server/src/db.rs` (new query + tests), `crates/server/src/web.rs` (new API handler, `trends_page` extended), `crates/server/src/main.rs` (new route).
- No new dependencies, no schema migration — reads `task_executions` as-is.
- No CLI changes.
