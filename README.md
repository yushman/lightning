# lightning

lightning is a self-hosted, open-source observability and acceleration platform for Gradle CI (Android monorepos first): a layer above the build, never inside it (see `docs/design/lightning-platform.md`). It currently ships four features: a **flaky-test radar** (a CLI uploads JUnit XML results from CI; the server tracks test history, computes deterministic flaky scores, and shows what flakes, since when, and on which commit), **build telemetry** ("build scans lite": a Gradle init script reports per-task timings, cache outcomes, and configuration/total build time to the same server), a **remote Gradle build cache** with analytics (the server speaks Gradle's HTTP build cache protocol and shows hit rates, storage stats, and never-cached tasks on top of the telemetry), and **selective execution** (`lightning sync`/`affected`/`run`: snapshot the module graph once, then decide what a diff touches in pure Rust — before any JVM starts).

## Run the server

```sh
cargo build --release
./target/release/lightning-server --addr 0.0.0.0:8080 --db lightning.db --retention-days 90
```

Flags are also available as env vars: `LIGHTNING_ADDR`, `LIGHTNING_DB`, `LIGHTNING_RETENTION_DAYS`. The UI is at `/` (flaky list), `/tests/{id}` (test history), `/runs/{id}` (run summary), `/builds` (builds list), `/builds/{id}` (build detail), `/trends` (per-branch build trends), `/cache` (cache storage and analytics); JSON at `/api/flaky` and `/api/builds`.

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

## Use the remote build cache

The server implements Gradle's HTTP build cache protocol at `/cache/{key}`. Enable it in `settings.gradle(.kts)` — push from CI, pull everywhere:

```groovy
// settings.gradle
buildCache {
    remote(HttpBuildCache) {
        url = 'https://lightning.example.com/cache/'   // trailing slash required
        push = System.getenv('CI') != null
        credentials {
            username = 'ci'                            // ignored by the server
            password = System.getenv('LIGHTNING_CACHE_TOKEN') ?: ''
        }
    }
}
```

Run builds with `--build-cache` (or `org.gradle.caching=true`). Storage is bounded and self-maintaining: artifacts land in a directory next to the db (`--cache-dir` / `LIGHTNING_CACHE_DIR`), single artifacts over 100 MiB are rejected (`--cache-max-artifact-mb`), the total is capped at 10 GiB with least-recently-used eviction (`--cache-max-size-mb`), and entries unused for 30 days are pruned (`--cache-retention-days`).

Writes can be protected with a shared token: start the server with `LIGHTNING_CACHE_TOKEN=<secret>` (or `--cache-token`) and give CI the same value — Gradle sends it as the Basic-auth password, the username is ignored. Reads stay open; without a token, writes are open too. Cache analytics (hit rate, top artifacts, never-cached task paths) live at `/cache` and improve as build telemetry accumulates.

## Run only what a diff affects

Selective execution needs no server. `lightning sync` runs Gradle once with an embedded init script and writes `lightning.lock`: modules, their declared source-set dirs (including out-of-module dirs like `srcDir("../shared")`), task names, and dependency edges typed `main`/`test`. The hot path never starts a JVM:

```sh
lightning sync                      # once, and whenever build files change
lightning affected                  # one affected module path per line
lightning affected --json           # + per-module reasons and merge-base
lightning run test -- --continue    # gradle :m:test for affected modules only
```

The diff is `merge-base(base, HEAD)..HEAD` plus uncommitted changes (disable with `--no-uncommitted`). The base ref defaults to `origin/main`; override with `--base <ref>`, or `--base-sha <sha>` when CI already knows the exact commit. Shallow clones are detected and reported (use `fetch-depth: 0`).

A module is affected when it contains changed files, when a changed module is reachable from it through `main` edges (transitively), or when one of its direct `test` edges (`testImplementation` and friends) points into that set — test edges never propagate further. Anything ambiguous — a file outside every module, a composite build — selects **everything**: a false negative is never an option, an extra run is.

The lock is invalidated by a hash over all build files (`**/*.gradle(.kts)`, `buildSrc/**`, `build-logic/**`, version catalog, wrapper and properties files); a stale lock aborts with exit code 4 unless you pass `--auto-sync`. Cache `lightning.lock` in CI keyed by that file set, or commit it — both work.

Optional `lightning.toml` next to the lock:

```toml
[affected]
base = "origin/main"       # default base ref
ignore = ["docs/**"]       # opt-in: exclude paths from the diff (no defaults)
invalidate_on = ["ci/**"]  # extra lock-invalidation globs
```

`lightning run <task>` matches the task name exactly against each affected module's task list (an Android-flavored module runs `testDebugUnitTest` even though a JVM module runs `test`), skips modules lacking it with a note, and exits with Gradle's exit code — or 0 without starting Gradle when nothing is affected.

Fast-exit a docs-only PR before checkout even finishes installing a JDK (exit code 3 = nothing affected, 0 = something, 4 = stale lock):

```yaml
- run: |
    if lightning affected --quiet --auto-sync; then
      echo "run=true" >> "$GITHUB_OUTPUT"
    fi
```

Or fan out heavy runners over affected modules only, with `lightning affected --format github-matrix`:

```yaml
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.plan.outputs.matrix }}
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - id: plan
        run: echo "matrix=$(lightning affected --format github-matrix --auto-sync)" >> "$GITHUB_OUTPUT"
  test:
    needs: plan
    if: ${{ fromJSON(needs.plan.outputs.matrix).include[0] }}
    strategy:
      matrix: ${{ fromJSON(needs.plan.outputs.matrix) }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: ./gradlew ${{ matrix.module }}:test
```
