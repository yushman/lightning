# Tasks

- [x] 1. Remove the diff-based staleness branch in `crates/cli/src/affected.rs::select`; staleness = missing lock / wrong version / hash mismatch only
- [x] 2. Add regression test: build file modified + re-synced → `affected` proceeds (no exit 4)
- [x] 3. Verify existing stale-lock tests still pass (hash path untouched)
- [x] 4. README.md / README_RU.md: extract telemetry init script to a temp path in the CI snippet
- [x] 5. Add `lightning.toml` to the invalidation hash set (sync.rs) and exclude it from the diff (gitdiff.rs, alongside lightning.lock)
- [x] 6. Quality gates: fmt, clippy -D warnings, cargo test --workspace
- [x] 7. E2E on nowinandroid: uncommitted build-file change + fresh sync → source-file change selects the closure (not exit 4 and not everything unless the build file itself is in the diff); build-logic change without re-sync → exit 4; lightning.toml present → no "outside all modules" degradation

## E2E record (nowinandroid @ 7d45eae4, Gradle 9.6.1, JDK 21)

Before the fix: uncommitted `settings.gradle.kts` (or an extracted `lightning.init.gradle` in the repo root) made every `affected` invocation exit 4 ("the diff touches build file ...") even immediately after `lightning sync`; an untracked `lightning.toml` degraded every selection to everything-affected (44 modules) via "outside all modules".

After: with untracked `lightning.toml` (`ignore = ["*.md", "docs/**"]`) present — `sync` (45 modules) → touch `core/model/src/**` → `affected` exit 0, 24 modules incl. `:core:model`, zero warnings; docs-only README.md change → `--quiet` exit 3; `build-logic/convention/build.gradle.kts` change → exit 4 via hash mismatch. Gates: fmt --check, clippy -D warnings, 31+2+23 tests green.
