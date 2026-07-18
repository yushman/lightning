## ADDED Requirements

### Requirement: Cache analytics screen
The server SHALL serve a screen at `/cache` showing: cache storage statistics (entry count, total size used, configured size cap, artifact size limit, retention days), the top stored artifacts by hit count (key, size, hits, created, last accessed), the overall task cache hit rate from telemetry (`from-cache` over `from-cache` + `success` + `failed` task executions) with a note that the denominator includes non-cacheable tasks, and a never-cached table: task paths executed at least 3 times across the recent builds window with no `from-cache` or `up-to-date` outcome, ordered by total execution time, with a note that the heuristic cannot distinguish uncacheable tasks from tasks whose inputs always change.

#### Scenario: Storage stats and hit rate shown
- **WHEN** cache entries and telemetry builds exist and `/cache` is fetched
- **THEN** the page shows entry count, total size, top artifacts with hit counts, and the overall hit rate

#### Scenario: Never-cached task listed
- **WHEN** a task path was executed in at least 3 recent builds and never resulted in `from-cache` or `up-to-date`
- **THEN** the page lists that path with its execution count and total execution time

## MODIFIED Requirements

### Requirement: Build detail screen
The server SHALL serve a screen at `/builds/{id}` showing a build's metadata (SHA, branch, outcome, Gradle/JDK versions, CI link when present), configuration versus execution time, a task outcome breakdown with counts per outcome, the build's cache hit rate (`from-cache` over `from-cache` + `success` + `failed` tasks), and the slowest tasks with their outcomes and durations.

#### Scenario: Slow tasks visible
- **WHEN** a build's detail page is fetched
- **THEN** the page shows configuration and total times, outcome counts, and the slowest tasks ordered by duration

#### Scenario: Build hit rate visible
- **WHEN** a build with `from-cache` and executed tasks is fetched at `/builds/{id}`
- **THEN** the page shows the build's cache hit rate

### Requirement: Section navigation
Every UI page SHALL include navigation links reaching the flaky tests list, the builds list, the trends screen, and the cache analytics screen.

#### Scenario: Builds reachable from flaky radar
- **WHEN** the flaky list at `/` is fetched
- **THEN** the page contains links to `/builds`, `/trends`, and `/cache`
