# Tasks

- [ ] 1. Remove the diff-based staleness branch in `crates/cli/src/affected.rs::select`; staleness = missing lock / wrong version / hash mismatch only
- [ ] 2. Add regression test: build file modified + re-synced → `affected` proceeds (no exit 4)
- [ ] 3. Verify existing stale-lock tests still pass (hash path untouched)
- [ ] 4. README.md / README_RU.md: extract telemetry init script to a temp path in the CI snippet
- [ ] 5. Quality gates: fmt, clippy -D warnings, cargo test --workspace
- [ ] 6. E2E on nowinandroid: uncommitted `settings.gradle.kts` change + fresh sync → source-file change selects the closure (not exit 4); build-file change without re-sync → exit 4
