## 1. Foundations

- [x] 1.1 `lightning.toml` config module (`[affected] base/ignore/invalidate_on`, deny unknown keys, defaults on missing file); tests
- [x] 1.2 Lock model (`lightning.lock` JSON: version, hash, unsupported, modules with dir/source_dirs/tasks/typed deps), deterministic serialization, load/save; tests
- [x] 1.3 Invalidation hash: fixed glob set + config extras, blake3 over sorted path+content, skip `.git`/`.gradle`/`build`; tests

## 2. Sync

- [x] 2.1 Groovy sync init script (separate asset, `include_str!`): `projectsEvaluated` dump (root build only) of paths, dirs, source-set dirs, task names, declared project deps per configuration, included-builds detection
- [x] 2.2 `lightning sync`: locate gradle root, run wrapper/gradle with init script + dump property, normalize dump into sorted lock with hash; unit tests for normalization and edge typing (closed test-config list)

## 3. Affected

- [x] 3.1 Git diff module: merge-base(base, HEAD) semantics, `--base`/`--base-sha`/config base, working-tree changes by default with `--no-uncommitted`, shallow-clone diagnostics, repo-root→lock-dir path relativization
- [x] 3.2 Mapping + closure: ignore globs, source-dir-first then longest-dir-prefix (root excluded), outside-file → everything, typed-edge closure (main transitive, test one hop into main-affected); unit tests
- [x] 3.3 Property test: random DAGs + random diffs vs independent naive reference, superset invariants (hand-rolled deterministic RNG, 500 cases)
- [x] 3.4 `lightning affected` command: staleness check (hash + paranoid glob) with exit 4 / `--auto-sync`, outputs text/`--json`/`--format github-matrix`/`--quiet` (exit 3), everything-affected degradations; tests

## 4. Run

- [x] 4.1 `lightning run <task>`: affected → exact task-name match per module, skip notes, single Gradle invocation with `:module:task` paths + args after `--`, propagate exit code, nothing-affected fast path

## 5. Fixtures, docs, verification

- [x] 5.1 Fixture Gradle project `tests/fixtures/multimodule`: core/lib/app main chain, fixtures module with a test edge from lib, `srcDir("../shared")`, `custom` module with registered `testDebugUnitTest` and no `test` task, docs dir + lightning.toml ignore
- [x] 5.2 README: sync/affected/run usage, exit codes, lightning.toml, GHA matrix fan-out + fast-exit examples
- [x] 5.3 Quality gates green: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (48 tests)
- [x] 5.4 E2E with real Gradle (record below): scenarios (a)–(f) all pass
- [x] 5.5 Phase 1–3 regression: workspace tests green, server smoke (`/`, `/builds`, `/trends`, `/cache`, `/runs/1`, run ingest, cache PUT/GET)

## E2E record

Executed 2026-07-18 with **real Gradle 9.6.1** (cached distribution) on JDK 21 (Corretto), debug binaries. Fixture `tests/fixtures/multimodule` copied into a scratch git repo (branch `main`): modules `:core` (java-library, `srcDir('../shared/src/main/java')`), `:lib` (api → `:core`, testImplementation → `:fixtures`), `:app` (implementation → `:lib`), `:fixtures` (java-library), `:custom` (no java plugin, custom `tooling` configuration → `:core`, registered `testDebugUnitTest`), plus `docs/` and `lightning.toml` with `ignore = ["docs/**"]`.

**Sync**: `lightning sync` ran `gradle --init-script ... help -q` and wrote `lightning.lock` with 6 modules. Verified in the lock: `:core.source_dirs` contains `shared/src/main/java` (out-of-module srcDir caught); edges `:lib→:core main`, `:lib→:fixtures test`, `:app→:lib main`, `:custom→:core main` (custom configuration typed main per closed test list); `:custom.tasks` contains `testDebugUnitTest` and no `test`. Second sync on the unchanged tree produced a **byte-identical** lock.

**(a) Leaf change closure + merge-base + typed edges.** Branch `feat-core` changed `core/src/main/java/core/Core.java`; `main` then advanced with a `fixtures/` change (to prove merge-base, not tip diff). `affected --base main` on `feat-core` → exactly `:app :core :custom :lib`, exit 0 — `:fixtures` correctly excluded (main's own change ignored). Branch `feat-fixtures` changed `fixtures/` → `affected --json` → `:fixtures reason=changed`, `:lib reason=test-dependency`, and **`:app` (main-dependent of `:lib`) not selected** — test edges do not propagate.

**(b) Docs-only fast-exit.** Branch `docs-only` changed `docs/readme.md`; `affected --base main --quiet` → exit **3**, no output; non-quiet → empty output, exit 0. `--quiet` on `feat-core` → exit 0.

**(c) Stale lock.** Branch `buildfile` committed a comment to `app/build.gradle`: `affected --base main` → `error: lightning.lock is stale (build files changed since sync) — run lightning sync (or pass --auto-sync)`, exit **4**; with `--auto-sync` → re-synced and printed `:app`, exit 0.

**(d) Selective run.** On `feat-core`: `run test --base main` → `:custom has no task "test", skipped`, invoked `gradle :app:test :core:test :lib:test`, BUILD SUCCESSFUL (real compilation of the project-dependency chain), exit 0 — `:fixtures:test` not requested. `run testDebugUnitTest --base main` → app/core/lib skipped, only `:custom:testDebugUnitTest` ran (`custom module tests` printed). On `docs-only`: `run test` → `nothing affected, skipping gradle`, exit 0, no Gradle process. Broken java + `run build -- --quiet` → Gradle's failure exit code 1 propagated.

**(e) File outside modules.** Untracked `ci-config.yml` at the root → warning `changed file ci-config.yml is outside all modules; selecting everything` and all 6 module paths (incl. `:`), exit 0; with `--no-uncommitted` the untracked file is ignored → empty. Uncommitted edit to `lib/src/...` (no commit) → `:app :lib` (working tree included by default).

**(f) GitHub matrix.** `affected --base main --format github-matrix` on `feat-core` → single-line `{"include":[{"module":":app"},{"module":":core"},{"module":":custom"},{"module":":lib"}]}` — parsed as valid JSON by python3.

**Extras.** Composite build (copy with `includeBuild 'included'`): sync warns and marks the lock unsupported; `affected --auto-sync` → warning + all modules, exit 0. Shallow clone (`--depth 1`, divergent refs fetched shallow): `affected --base origin/main` → actionable error naming the shallow clone, `fetch-depth: 0` / `--unshallow` / `--base-sha`, exit 1; `--base-sha <sha>` then worked in the same clone. Unknown base ref in a full repo → `base ref "origin/nonexistent" is unknown — fetch it ...`, exit 1.

**Regression (phases 1–3)**: all 48 workspace unit tests green (25 cli + 23 server); server smoke on `127.0.0.1:4250`: `POST /api/runs` → 201, `GET /` `/builds` `/trends` `/cache` `/runs/1` → 200, cache `PUT`/`GET` roundtrip → 201/200. fmt/clippy/test re-run clean on the final tree. Server and Gradle daemons killed after verification.
