## Why

Phases 1–3 observe and cache builds; CI still runs every module's tasks on every PR. Phase 4 of the lightning platform adds selective execution: compute which Gradle modules a diff actually touches and run only their tasks — without paying Gradle configuration time on the hot path, so a docs-only PR can exit in seconds before any JVM starts.

## What Changes

- `lightning sync`: runs Gradle once with an embedded init script (Groovy, same embedding pattern as the telemetry script, separate asset) that dumps the project model; the CLI normalizes it into `lightning.lock` — deterministic JSON with modules (path, dir), declared source-set dirs (repo-root-relative, catches `srcDir("../shared")`), per-module task names, inter-module dependency edges typed `main`/`test`, and a blake3 hash of the build-file set for invalidation.
- `lightning affected`: pure-Rust hot path — git diff against `merge-base(base, HEAD)` (default base `origin/main`, config/flag override, `--base-sha` escape hatch, actionable shallow-clone errors) plus uncommitted changes by default; maps files to modules via source-set dirs then longest module-dir prefix; computes the affected closure over typed edges (main edges propagate transitively, test edges only one hop); outputs text, `--json`, `--format github-matrix`, and `--quiet` exit-code mode (0 affected / 3 nothing). Stale lock (build-file hash mismatch, or diff touching an invalidation glob) is refused with exit 4 unless `--auto-sync`.
- `lightning run <task>`: computes affected, invokes `./gradlew` (or `gradle`) with `:module:task` for each affected module whose task list contains the task (exact match; others skipped with a note), passes through extra args after `--`, propagates Gradle's exit code; nothing affected → exit 0 without invoking Gradle.
- `lightning.toml` config: `[affected]` with `base`, `ignore` (opt-in globs excluded from the diff, no defaults), `invalidate_on` (extra invalidation globs).
- False-negative-never invariant: every ambiguous situation (file outside all modules, missing/stale lock without `--auto-sync` is refused, composite builds, unknown edge kinds) degrades to "everything affected". Guarded by a property test comparing the closure against an independent naive reference on random DAGs and diffs.
- Committed fixture Gradle projects under `tests/fixtures/` and README docs (usage, GitHub Actions matrix fan-out, fast-exit example).

## Capabilities

### New Capabilities

- `affected-selection`: the lock model and sync flow, invalidation hashing, diff/base semantics, file→module mapping, typed-edge closure, output formats and exit codes, `run` task fan-out, config keys, and the everything-affected degradations.

### Modified Capabilities

- None. The server, ingest, cache, and existing CLI commands are untouched; affected computation is CLI-only by design.

## Impact

- CLI: new subcommands `sync`, `affected`, `run`; new modules (config, lock, sync, gitdiff, affected, run); new embedded asset `lightning.sync.init.gradle`; new dependency `toml`.
- Repo: fixture Gradle projects under `tests/fixtures/`; README gains a selective-execution section.
- Server: no changes.
