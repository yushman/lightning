# build-telemetry Specification

## Purpose
TBD - created by archiving change add-build-telemetry. Update Purpose after archive.
## Requirements
### Requirement: Init script extraction command
The CLI SHALL provide `lightning init-script` which emits the Gradle init script embedded in the binary. With `--out <path>` it SHALL write the script to that path, overwriting an existing file; without `--out` it SHALL print the script to stdout.

#### Scenario: Extract to file
- **WHEN** `lightning init-script --out lightning.init.gradle` runs
- **THEN** the file contains the embedded Groovy init script and the command exits zero

#### Scenario: Print to stdout
- **WHEN** `lightning init-script` runs without `--out`
- **THEN** the script is written to stdout

### Requirement: Build event collection via public Gradle API
The init script SHALL collect telemetry using only public Gradle APIs: a `BuildService` implementing `OperationCompletionListener`, registered through `BuildEventsListenerRegistry`. For every executed task it SHALL record the task path, wall-clock duration, and outcome classified as `success`, `up-to-date`, `from-cache`, `failed`, or `skipped`. It SHALL also record configuration time (build start to first task start), total build time (build start to build finish), requested tasks, Gradle version, JDK version, git SHA and branch (from CI environment variables or the local git repository, absent when undeterminable), and CI run URL when derivable.

#### Scenario: Task outcomes classified
- **WHEN** a build runs tasks that succeed, are up-to-date, come from the build cache, fail, or are skipped
- **THEN** the payload reports each task with the corresponding outcome and its duration

#### Scenario: Build-level timings
- **WHEN** a build finishes
- **THEN** the payload contains total build time and configuration time along with requested tasks and Gradle/JDK versions

### Requirement: Publishing at build finish
The init script SHALL POST the collected payload as one JSON document to `<server>/api/builds` when the build finishes. The server URL SHALL be resolved from the Gradle property `lightning.url`, else the environment variable `LIGHTNING_URL`. Each build invocation SHALL carry a unique `build_key` so that re-posting the same payload is idempotent while two separate builds are always two entries.

#### Scenario: URL from environment
- **WHEN** `LIGHTNING_URL` is set and a build finishes
- **THEN** the script POSTs the payload to `$LIGHTNING_URL/api/builds`

#### Scenario: Distinct builds stay distinct
- **WHEN** the same task set is built twice with telemetry enabled
- **THEN** two payloads with different `build_key` values are posted

### Requirement: Fail-safe telemetry
Telemetry SHALL never break a build: any error during registration, collection, metadata resolution, or publishing SHALL be caught, logged as a warning, and swallowed. HTTP requests SHALL have bounded timeouts and a single attempt. When no server URL is configured the script SHALL do nothing beyond logging.

#### Scenario: Server unreachable
- **WHEN** the configured server is down and a build runs with the init script
- **THEN** the build completes with its normal result and only a warning is logged

#### Scenario: No URL configured
- **WHEN** neither `lightning.url` nor `LIGHTNING_URL` is set
- **THEN** the build runs normally and nothing is posted

