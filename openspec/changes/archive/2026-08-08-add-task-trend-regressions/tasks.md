## 1. Query layer

- [x] 1.1 Move `median(Vec<i64>) -> Option<i64>` from `web.rs` (private) to `db.rs` as `pub(crate) fn median`; update `web.rs::trends_page` to import it from `db.rs` instead of defining its own copy
- [x] 1.2 In `db.rs`, add `REGRESSION_RECENT_WINDOW`/`REGRESSION_BASELINE_WINDOW`-parameterized query (or Rust-side split of a fetched build list, mirroring `never_cached_tasks`'s builds-window CTE) that returns per task path, per branch: recent execution count, recent median, baseline execution count, baseline median — filtered to `success`/`failed` outcomes only, using the shared `median` helper
- [x] 1.3 Add a `TaskRegression` struct (path, baseline_median_ms, recent_median_ms, pct_delta) and a `task_regressions(conn, branch, recent_window, baseline_window, min_executions, threshold_pct, min_delta_ms) -> Vec<TaskRegression>` function applying the classification rule (min executions in both windows, relative AND absolute threshold), sorted by pct_delta descending
- [x] 1.4 Unit tests: regressed by both thresholds; relative-only (below absolute floor) not flagged; below min-executions not flagged; task absent from baseline window excluded; median computed correctly with even/odd counts

## 2. API endpoint

- [x] 2.1 Add `GET /api/trends/regressions` handler in `web.rs`, reading `branch` query param (default `main`), calling `task_regressions` with the constants from design.md, returning JSON array
- [x] 2.2 Register the route in `main.rs`
- [x] 2.3 No dedicated handler-level test added — `web.rs` has zero existing tests for any of its handlers (`flaky_api`, `builds_api`, `trends_page`, etc.), all logic is unit-tested at the `db.rs` query layer per existing convention; `regressions_api` is a thin passthrough to `db::task_regressions` (already covered by 1.4) plus the `branch` default, verified by reading and by `cargo build`/`clippy`

## 3. Trends page

- [x] 3.1 Extend `trends_page` to render a "Regressed tasks" table per branch beneath the existing summary table, reusing the branch grouping already computed there
- [x] 3.2 Explicit empty-state text ("No tasks have regressed on this branch.") when a branch has zero regressed tasks
- [x] 3.3 No `_page` handler in `web.rs` has a dedicated test (see 2.3); rendering verified by reading the generated `format!` templates and by `cargo build`/`clippy`, consistent with how `trends_page`'s existing build-duration table is (not) tested today

## 4. Wrap-up

- [x] 4.1 Update `openspec/specs/build-telemetry` cross-references if any docs mention `/trends` scope (check `README.md`/`README_RU.md` for a `/trends` description to update)
- [x] 4.2 Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` for the workspace
