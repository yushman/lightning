## Context

`task_executions` (build_id, path, outcome, duration_ms) already holds everything needed; `db.rs` already has two precedents for window-based aggregation over it: `never_cached_tasks` (a builds-window join, `success`/`failed` as the "real work" filter, `min_executions` floor) and `trends_page`'s per-branch median-of-builds grouping. This change is the same shape applied per task path instead of per build, with two windows instead of one.

## Goals / Non-Goals

**Goals:**
- Detect a task that has gotten durably slower, using only data already collected, with a rule simple enough to recompute by hand from two numbers and a threshold — matching the flaky-scoring precedent of "no ML, no black box."
- Reuse the existing branch-grouping approach in `web.rs`/`db.rs` rather than inventing a parallel path; share one median implementation between `trends_page`'s build-duration medians and the new task-duration medians instead of having two copies of the same sort-and-index logic.

**Non-Goals:**
- Statistical trend detection (regression lines, change-point detection, stddev-based anomaly scores). A two-window median comparison with a percentage-plus-absolute-floor rule is deliberately blunter, for the same reason flaky scoring rejected ML: every number in the answer has to be checkable by a human reading the requirement text.
- Cross-branch comparison (e.g. PR branch vs `main`). Both windows are drawn from the same branch's own history; comparing a feature branch's five builds against `main`'s baseline is a different, noisier question left for later.
- Alerting/notification. This change only exposes the data (API + page); wiring it into `add-pr-annotations`-style PR comments is a natural follow-up, not bundled here to keep the two changes independently reviewable.

## Decisions

- **10-build recent / 30-build baseline, adjacent and non-overlapping**: chosen to echo the existing constants' order of magnitude (`TREND_WINDOW = 50`, `NEVER_CACHED_WINDOW = 100`) while keeping the recent window tight enough to catch a regression within roughly a day or two of CI activity on an active branch, and the baseline wide enough that one noisy build doesn't move it much. These become named constants (`REGRESSION_RECENT_WINDOW`, `REGRESSION_BASELINE_WINDOW`) next to `TREND_WINDOW` in `web.rs`, not hardcoded.
- **25% relative + 500ms absolute, both required**: a relative-only threshold flags trivial tasks (20ms → 30ms) that are pure noise; an absolute-only threshold misses meaningful regressions on already-slow tasks (10s → 12s is a real 2s regression at only 20%). Requiring both is the same two-part-guard shape as flaky scoring's `flip_shas >= 1 OR flips >= 2` — a rule stated once in the spec and directly checkable against the two medians.
- **Median, not mean**: consistent with `trends_page`'s existing `median()` helper (already used for build durations and avoided-percent) and with the project's stated aversion to a single outlier build skewing the signal.
- **One SQL query per branch, computed in Rust like `never_cached_tasks`, not two window functions in SQL**: `rusqlite`'s SQLite build may lack `PERCENTILE_CONT`; `web.rs` already has a private `median(Vec<i64>) -> Option<i64>` helper doing sort-and-index-into-middle for build-duration trends. This change moves it to `db.rs` as `pub(crate) fn median`, since the query layer is where it's needed now, and `web.rs::trends_page` imports it from there — one implementation, two call sites, instead of a second copy living next to the new query.
- **New `/api/trends/regressions` endpoint rather than extending `/api/flaky`'s shape**: regressions and flaky tests are different domains (durations vs. pass/fail verdicts) sharing only "deterministic anomaly over a window" as a concept; a separate endpoint keeps `flaky_api`'s response shape stable for any existing consumer.

## Risks / Trade-offs

- [Risk] A branch with fewer than 40 builds total never has both a full recent and full baseline window, so nothing is ever flagged early in a branch's life. → Mitigation: acceptable — the `min_executions >= 3` per window (not per build count) means a low-volume branch degrades to "nothing evaluated" rather than a misleading answer from a near-empty baseline; this mirrors `never_cached_tasks`'s own floor.
- [Risk] A one-time slow build (e.g. a noisy CI runner) inside the recent window can nudge its median if the window is small enough. → Mitigation: median (not mean) already resists a single outlier; the 10-build recent window is wide enough that one bad build moves the median only if multiple neighbors are also elevated.
- [Risk] Task paths get renamed (module renames, Gradle task renames) and silently "reset" — a renamed task looks like a brand-new task absent from the baseline window rather than a continuation. → Mitigation: same limitation `never_cached_tasks` already accepts (paths are the only identity available); out of scope to fix here.

## Open Questions

- None blocking. Whether to add a "regression cleared" transition (task was regressed, now isn't) as its own signal is left for later — the current design is stateless per request, recomputed from scratch each time, so a cleared regression simply stops appearing.
