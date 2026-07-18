## 1. Init script

- [x] 1.1 Groovy init script at `crates/cli/assets/lightning.init.gradle`: `Plugin<Gradle>` with injected `BuildEventsListenerRegistry`, `BuildService` + `OperationCompletionListener` collecting task path/duration/outcome; timing model per design D3
- [x] 1.2 Payload assembly in `close()`: build_key (UUID), outcome, requested tasks, Gradle/JDK versions, git SHA/branch (env then `git rev-parse`), CI URL; POST via `HttpURLConnection` with timeouts
- [x] 1.3 Fail-safe wrapping: try/catch around registration and publishing, warning log prefix `lightning:`, no-op without URL

## 2. CLI: lightning init-script

- [x] 2.1 `init-script` subcommand: `--out <path>` writes embedded script, stdout otherwise; unit test that emitted content matches the asset

## 3. Server: build ingest and storage

- [x] 3.1 Schema: `builds` + `task_executions` tables and index created at startup
- [x] 3.2 `POST /api/builds`: transactional insert, outcome validation, 400 on malformed, dedup by build_key with indicator; tests
- [x] 3.3 Retention pruning extended to builds; test
- [x] 3.4 `GET /api/builds` JSON list newest first with outcome counts

## 4. Web UI

- [x] 4.1 Shared nav (flaky · builds · trends) in the page shell; duration formatter
- [x] 4.2 `/builds` list with outcome, duration, avoidance ratio
- [x] 4.3 `/builds/{id}` detail: metadata, configuration vs execution, outcome breakdown, slowest tasks
- [x] 4.4 `/trends` per-branch medians over recent builds

## 5. Verification and docs

- [x] 5.1 Quality gates green: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [x] 5.2 E2E with real Gradle: fixture project in scratchpad, server on a local port, build twice with `--init-script` (second build UP-TO-DATE), verify builds/tasks/timings/outcomes via API and UI pages with curl; re-POST captured payload to verify build_key dedup; record commands and results below
- [x] 5.3 Phase-1 regression: existing tests pass, `/` still renders
- [x] 5.4 README: init-script CI integration snippet

## E2E record

Executed 2026-07-18 with **real Gradle 9.6.1** (distribution downloaded from services.gradle.org, SHA-256 verified `9c0f7fae…`) on JDK 21 (Corretto), debug binaries. Fixture: minimal two-project Gradle build (`settings.gradle` + `:lib` java-library with one source file) in the scratchpad, its own git repo (sha `8651e7c`, branch `main`).

Commands:

```
lightning-server --addr 127.0.0.1:4242 --db $S/e2e/lightning.db &
lightning init-script --out $S/fixture/lightning.init.gradle
export LIGHTNING_URL=http://127.0.0.1:4242
gradle build --init-script lightning.init.gradle --no-daemon      # build 1: telemetry sent (201)
gradle build --init-script lightning.init.gradle --no-daemon      # build 2: telemetry sent (201)
gradle build --build-cache ... ; gradle clean ; gradle build --build-cache ...   # builds 3-8
echo 'garbage' >> Greeter.java && gradle build ...                # build 9 (BUILD FAILED)
LIGHTNING_URL=http://127.0.0.1:9 gradle help ...                  # dead server
gradle help --init-script lightning.init.gradle                   # no URL
gradle help ... -Plightning.url=http://127.0.0.1:4242             # property URL
```

Verified via HTTP:

- Two builds = two entries: `GET /api/builds` shows distinct UUID `build_key`s, `sha 8651e7c…`, `branch main`. Build 1: 5 success / 2 up-to-date / 4 skipped of 11 tasks, total 718ms, configuration 442ms. Build 2 (unchanged rebuild): 7 up-to-date / 4 skipped, 0 success — UP-TO-DATE outcomes captured.
- FROM-CACHE: after populating the local build cache and `clean`, the rebuild reported `:lib:compileJava FROM-CACHE`; `/api/builds` shows `from-cache: 1` distinct from `up-to-date: 3` (from-cache checked before up-to-date, since tooling API sets both flags).
- Failed build: compile error → Gradle exits `BUILD FAILED`, telemetry still sent; payload has `outcome: failed`, task `:lib:compileJava` failed; `/builds/9` renders `outcome failed` and the failed task.
- build_key dedup: same synthetic payload POSTed twice → `201 {"deduplicated":false}` then `200 {"deduplicated":true}`, build count unchanged.
- Malformed payloads (`outcome:"weird"`, missing fields) → `400`.
- UI via curl: `/builds` table with branch/sha/outcome/duration/avoided; `/builds/1` shows Gradle 9.6.1 / JDK 21.0.11, total/configuration/execution split, outcome breakdown, slowest-tasks table; `/trends` shows branch `main`, median duration, median avoided 63%, bar; `/builds/999` → 404.
- Fail-safe: dead server → `lightning: telemetry publish failed: java.net.ConnectException…` and `BUILD SUCCESSFUL`; no URL → `lightning: no server url … telemetry skipped` and `BUILD SUCCESSFUL`; `-Plightning.url=…` works as the env alternative.
- Phase-1 regression: `POST /api/runs` → 201, `/runs/1` renders `1 passed · 0 failed`, `/` renders (all 20 workspace unit tests green).

Server and Gradle daemons killed after verification. Quality gates re-run on the final tree: fmt/clippy/test all green.
