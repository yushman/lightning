# lightning

lightning is a self-hosted, open-source observability and acceleration platform for Gradle CI (Android monorepos first): a layer above the build, never inside it (see `docs/design/lightning-platform.md`). This first release is a **flaky-test radar**: a CLI uploads JUnit XML results from CI, and a single-binary server tracks test history, computes deterministic flaky scores, and shows what flakes, since when, and on which commit.

## Run the server

```sh
cargo build --release
./target/release/lightning-server --addr 0.0.0.0:8080 --db lightning.db --retention-days 90
```

Flags are also available as env vars: `LIGHTNING_ADDR`, `LIGHTNING_DB`, `LIGHTNING_RETENTION_DAYS`. The UI is at `/` (flaky list), `/tests/{id}` (test history), `/runs/{id}` (run summary); JSON at `/api/flaky`.

## Add upload to CI

Add one step after your tests (no build integration needed):

```yaml
- name: Upload test results to lightning
  if: always()
  run: lightning upload --server https://lightning.example.com
```

`lightning upload` parses reports matching `**/build/test-results/**/*.xml` (override with `--glob`), takes SHA/branch and run identity from GitHub Actions env or the local git repo, and uploads idempotently — re-running the step never duplicates a run.
