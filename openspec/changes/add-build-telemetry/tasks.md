## 1. Init script

- [ ] 1.1 Groovy init script at `crates/cli/assets/lightning.init.gradle`: `Plugin<Gradle>` with injected `BuildEventsListenerRegistry`, `BuildService` + `OperationCompletionListener` collecting task path/duration/outcome; timing model per design D3
- [ ] 1.2 Payload assembly in `close()`: build_key (UUID), outcome, requested tasks, Gradle/JDK versions, git SHA/branch (env then `git rev-parse`), CI URL; POST via `HttpURLConnection` with timeouts
- [ ] 1.3 Fail-safe wrapping: try/catch around registration and publishing, warning log prefix `lightning:`, no-op without URL

## 2. CLI: lightning init-script

- [ ] 2.1 `init-script` subcommand: `--out <path>` writes embedded script, stdout otherwise; unit test that emitted content matches the asset

## 3. Server: build ingest and storage

- [ ] 3.1 Schema: `builds` + `task_executions` tables and index created at startup
- [ ] 3.2 `POST /api/builds`: transactional insert, outcome validation, 400 on malformed, dedup by build_key with indicator; tests
- [ ] 3.3 Retention pruning extended to builds; test
- [ ] 3.4 `GET /api/builds` JSON list newest first with outcome counts

## 4. Web UI

- [ ] 4.1 Shared nav (flaky · builds · trends) in the page shell; duration formatter
- [ ] 4.2 `/builds` list with outcome, duration, avoidance ratio
- [ ] 4.3 `/builds/{id}` detail: metadata, configuration vs execution, outcome breakdown, slowest tasks
- [ ] 4.4 `/trends` per-branch medians over recent builds

## 5. Verification and docs

- [ ] 5.1 Quality gates green: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] 5.2 E2E with real Gradle: fixture project in scratchpad, server on a local port, build twice with `--init-script` (second build UP-TO-DATE), verify builds/tasks/timings/outcomes via API and UI pages with curl; re-POST captured payload to verify build_key dedup; record commands and results below
- [ ] 5.3 Phase-1 regression: existing tests pass, `/` still renders
- [ ] 5.4 README: init-script CI integration snippet

## E2E record

(to be filled during verification)
