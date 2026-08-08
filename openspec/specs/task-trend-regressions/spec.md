# task-trend-regressions Specification

## Purpose
TBD - created by archiving change add-task-trend-regressions. Update Purpose after archive.
## Requirements
### Requirement: Baseline and recent duration windows
For a given branch, the system SHALL compute, per task path, the median duration over the **recent window** (the last 10 builds on that branch) and the median duration over the **baseline window** (the 30 builds immediately preceding the recent window on that branch), counting only executions with outcome `success` or `failed` (the same "real work" filter `never_cached_tasks` uses; `up-to-date`, `from-cache`, and `skipped` executions are excluded as not comparable work).

#### Scenario: Task present in both windows
- **WHEN** a task path has at least one `success`/`failed` execution in both the recent and baseline windows on a branch
- **THEN** both medians are computed for that task path

#### Scenario: Task absent from the baseline window
- **WHEN** a task path has executions in the recent window but none in the baseline window (a new task)
- **THEN** the task is excluded from regression evaluation for that branch

### Requirement: Regression classification
A task path SHALL be classified regressed on a branch iff it has at least 3 qualifying executions in each of the recent and baseline windows, AND `recent_median > baseline_median * 1.25`, AND `recent_median - baseline_median >= 500` (milliseconds). Both the percentage and absolute-delta conditions SHALL hold; a large percentage change on a near-zero baseline (e.g. 20ms to 30ms) SHALL NOT be classified regressed.

#### Scenario: Regression by both thresholds
- **WHEN** a task's baseline median is 2000ms and its recent median is 3000ms, with at least 3 qualifying executions in each window
- **THEN** the task is classified regressed (50% increase, 1000ms absolute delta)

#### Scenario: Percentage threshold met but below the noise floor
- **WHEN** a task's baseline median is 100ms and its recent median is 200ms
- **THEN** the task is NOT classified regressed (100% increase, but only 100ms absolute delta)

#### Scenario: Below the minimum execution count
- **WHEN** a task has only 2 qualifying executions in the recent window
- **THEN** the task is excluded from regression evaluation regardless of its medians

### Requirement: Regressions API
The server SHALL expose `GET /api/trends/regressions?branch=<name>` returning regressed task paths for that branch as JSON, each with path, baseline median, recent median, and percent delta, ordered by percent delta descending. The `branch` query parameter SHALL default to `main` when omitted.

#### Scenario: Regressed tasks returned
- **WHEN** a branch has one or more regressed task paths
- **THEN** `GET /api/trends/regressions?branch=<name>` returns them as JSON, most-regressed first

#### Scenario: No branch specified
- **WHEN** the `branch` query parameter is omitted
- **THEN** the response covers the `main` branch

### Requirement: Regressed tasks on the trends page
The `/trends` page SHALL include a "Regressed tasks" section per branch, beneath the existing build-duration summary, listing each regressed task path with its baseline median, recent median, and percent delta. A branch with no regressed tasks SHALL show an explicit empty state rather than omitting the section.

#### Scenario: Branch with regressions
- **WHEN** a branch shown on `/trends` has regressed task paths
- **THEN** its section lists them with baseline median, recent median, and percent delta

#### Scenario: Branch with no regressions
- **WHEN** a branch shown on `/trends` has no regressed task paths
- **THEN** its section states explicitly that no tasks have regressed

