## 1. Sync and lock

- [x] 1.1 Init script: report included builds (name + root-relative dir); when present, run substitution detection over every resolvable configuration's resolution result (`ProjectComponentIdentifier` in a non-root build via `buildPath`/`isCurrentBuild`, fail safe on errors); plugin-only → no unsupported marker, substitution/unverifiable/outside-root → unsupported with reason
- [x] 1.2 Lock v2: `included_builds` field (sorted root-relative dirs), VERSION = 2; v1 locks rejected with a re-sync error; tests
- [x] 1.3 Sync: parse `included_builds` from the dump, normalize into the lock, include `<dir>/**` globs (glob-escaped) in the build-files hash; tests

## 2. Affected

- [x] 2.1 Staleness: hash recompute and paranoid matchers in `affected`/`run` include the lock's included-build globs; tests
- [x] 2.2 Compute: diffed file under an included-build root → everything with "build logic changed" reason (before outside-all-modules fallback); tests
- [x] 2.3 Root rule: exclude `:` from everything-affected listings unless it declares source dirs; tests

## 3. Fixtures and integration tests

- [x] 3.1 Fixture `tests/fixtures/composite-plugin-only`: `pluginManagement { includeBuild("gradle/conventions") }` (deliberately not named build-logic) with a precompiled convention plugin applied by a module; `tests/fixtures/composite-substituting`: module depends on `com.example:library` provided by `includeBuild("included-lib")`
- [x] 3.2 Integration test (skips when no `gradle` on PATH): plugin-only fixture syncs to a normal lock with `included_builds = ["gradle/conventions"]`; substituting fixture keeps the refusal with a substitution reason

## 4. Docs and verification

- [x] 4.1 README.md / README_RU.md: composite-build sentence reflects the new behavior
- [x] 4.2 Quality gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green (integration tests exercised with a real Gradle)
- [x] 4.3 Real-repo E2E on nowinandroid (record below): re-sync produces a normal lock recording `build-logic`; synthetic `core/model` change selects `:core:model` + reverse closure (not all 45); docs-only change exits 3 with `--quiet`; a `build-logic/` change triggers stale/paranoid handling and, with `--auto-sync`, the "build logic changed" everything reason; `:` absent from listings; working tree restored

## E2E record

Executed 2026-07-18 on the nowinandroid clone (JDK 21 Corretto, Android SDK via `local.properties`, Gradle 8.13 wrapper cached), release binary. Fixture integration tests additionally run against standalone **Gradle 9.4.0 and 8.13** (`cargo test -p lightning-cli --test composite` with each on PATH) — both green, covering the `buildPath` era and daemon variance.

**Reproduction (pre-fix binary).** `lightning affected --auto-sync` on nowinandroid → `warning: composite build (included builds: build-logic); selecting everything` and **45** modules printed, first line the root `:`.

**Detection mechanism note.** First implementation resolved graphs from `gradle.projectsEvaluated` and failed on Gradle 9.4 with "Resolution of the configuration ':app:compileClasspath' was attempted without an exclusive lock" — fail-safe correctly kept the refusal. Moved detection to `gradle.afterProject` (runs under the project's own lock); verified by probe and both fixtures on 9.4.0 and 8.13.

**v1 lock rejected.** With the old lock present: `error: .../lightning.lock has version 1, this build understands 2 — run lightning sync`.

**Setup.** Branch `lightning-test` (buildCache settings block committed there so the diff stays clean), `lightning.lock`/`lightning.init.gradle`/`lightning.toml` in `.git/info/exclude`, `lightning.toml` with `ignore = ["docs/**"]`.

**Sync.** `lightning sync` → `included builds without dependency substitution (plugin-only): build-logic`, `wrote lightning.lock (45 modules)` in **11.8 s** wall (substitution detection resolved every resolvable configuration of all 45 AGP/KMP modules without a single failure). Lock: `version: 2`, no `unsupported`, `included_builds: ["build-logic"]`.

**(a) Empty diff.** `affected --base-sha HEAD --quiet` → exit **3**.

**(b) Synthetic core/model change.** Comment appended to `core/model/src/.../UserData.kt` → `affected --base-sha HEAD` → **24** modules (`:core:model` + reverse closure: `:app`, `:app-nia-catalog`, `:benchmarks`, `:core:data*`, `:core:database`, `:core:datastore*`, `:core:domain`, `:core:network`, `:core:notifications`, `:core:testing`, `:core:ui`, `:feature:*`, `:sync:*`), **not** all 45; no everything warning; `--json` shows exactly `{":core:model", reason: changed}` and `everything: false`; root `:` absent.

**(c) Docs-only.** Change under `docs/` → `affected --base-sha HEAD --quiet` → exit **3**.

**(d) build-logic change.** Comment appended to `build-logic/convention/src/.../AndroidApplicationConventionPlugin.kt` → without `--auto-sync`: `error: lightning.lock is stale (build files changed since sync) — run lightning sync (or pass --auto-sync)`, exit **4** (dynamic `build-logic/**` in the hash set fired, not the outside-all-modules path). With `--auto-sync`: re-synced, then `warning: build logic changed: build-logic/convention/src/main/kotlin/AndroidApplicationConventionPlugin.kt is inside included build build-logic; selecting everything` → **44** modules, root `:` absent (bare root excluded from everything listings).

**(e) Dynamic invalidation with a non-build-logic name** proven by the fixture integration test: included build at `gradle/conventions`, a non-`.gradle` file added under it → exit 4 stale.

**Restore.** `git checkout main`, branch deleted, buildCache settings patch re-applied uncommitted, `lightning.toml` removed, `.git/info/exclude` additions reverted; `local.properties`, `lightning.lock` (fresh v2), `lightning.init.gradle` left in place. `git status`: only ` M settings.gradle.kts`.

**Gates.** `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` → 30 cli + 2 composite integration + 23 server, all green (integration tests run with real Gradle on PATH; they skip loudly when absent).
