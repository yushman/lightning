# lightning

lightning is a self-hosted, open-source observability and acceleration platform for Gradle CI (Android monorepos first): a layer above the build, never inside it (see `docs/design/lightning-platform.md`). It currently ships two features: a **flaky-test radar** (a CLI uploads JUnit XML results from CI; the server tracks test history, computes deterministic flaky scores, and shows what flakes, since when, and on which commit) and **build telemetry** ("build scans lite": a Gradle init script reports per-task timings, cache outcomes, and configuration/total build time to the same server).

## Run the server

```sh
cargo build --release
./target/release/lightning-server --addr 0.0.0.0:8080 --db lightning.db --retention-days 90
```

Flags are also available as env vars: `LIGHTNING_ADDR`, `LIGHTNING_DB`, `LIGHTNING_RETENTION_DAYS`. The UI is at `/` (flaky list), `/tests/{id}` (test history), `/runs/{id}` (run summary), `/builds` (builds list), `/builds/{id}` (build detail), `/trends` (per-branch build trends); JSON at `/api/flaky` and `/api/builds`.

## Add upload to CI

Add one step after your tests (no build integration needed):

```yaml
- name: Upload test results to lightning
  if: always()
  run: lightning upload --server https://lightning.example.com
```

`lightning upload` parses reports matching `**/build/test-results/**/*.xml` (override with `--glob`), takes SHA/branch and run identity from GitHub Actions env or the local git repo, and uploads idempotently — re-running the step never duplicates a run.

## Add build telemetry to CI

Extract the embedded Gradle init script and attach it to your build:

```yaml
- name: Enable lightning build telemetry
  run: lightning init-script --out lightning.init.gradle

- name: Build
  run: ./gradlew build --init-script lightning.init.gradle
  env:
    LIGHTNING_URL: https://lightning.example.com
```

The script collects per-task timings and outcomes (success / up-to-date / from-cache / failed / skipped), configuration and total build time, requested tasks, Gradle/JDK versions, and git/CI metadata, then POSTs one JSON document to `/api/builds` when the build finishes. The server URL comes from the `lightning.url` Gradle property (`-Plightning.url=...`) or the `LIGHTNING_URL` env var. Telemetry is fail-safe: without a URL it does nothing, and any error is logged and swallowed — it never breaks the build.
