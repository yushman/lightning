## ADDED Requirements

### Requirement: Builds list screen
The server SHALL serve a screen at `/builds` listing recent builds newest first, showing per build: time, branch, short SHA, outcome, requested tasks, total duration, and cache avoidance ratio (`up-to-date` + `from-cache` tasks over all tasks). Each row SHALL link to the build detail screen.

#### Scenario: Build listed with avoidance ratio
- **WHEN** a build with cached and executed tasks exists and `/builds` is fetched
- **THEN** the page shows the build with its duration and avoidance ratio linking to its detail page

### Requirement: Build detail screen
The server SHALL serve a screen at `/builds/{id}` showing a build's metadata (SHA, branch, outcome, Gradle/JDK versions, CI link when present), configuration versus execution time, a task outcome breakdown with counts per outcome, and the slowest tasks with their outcomes and durations.

#### Scenario: Slow tasks visible
- **WHEN** a build's detail page is fetched
- **THEN** the page shows configuration and total times, outcome counts, and the slowest tasks ordered by duration

### Requirement: Branch trend screen
The server SHALL serve a screen at `/trends` aggregating recent builds per branch: build count, median total duration of successful builds, and median cache avoidance ratio.

#### Scenario: Branch medians shown
- **WHEN** several builds exist on a branch and `/trends` is fetched
- **THEN** the page shows that branch with its build count and median duration

### Requirement: Section navigation
Every UI page SHALL include navigation links reaching the flaky tests list, the builds list, and the trends screen.

#### Scenario: Builds reachable from flaky radar
- **WHEN** the flaky list at `/` is fetched
- **THEN** the page contains links to `/builds` and `/trends`
