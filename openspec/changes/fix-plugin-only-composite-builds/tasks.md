## 1. Sync and lock

- [ ] 1.1 Init script: report included builds (name + root-relative dir); when present, run substitution detection over every resolvable configuration's resolution result (`ProjectComponentIdentifier` in a non-root build via `buildPath`/`isCurrentBuild`, fail safe on errors); plugin-only → no unsupported marker, substitution/unverifiable/outside-root → unsupported with reason
- [ ] 1.2 Lock v2: `included_builds` field (sorted root-relative dirs), VERSION = 2; v1 locks rejected with a re-sync error; tests
- [ ] 1.3 Sync: parse `included_builds` from the dump, normalize into the lock, include `<dir>/**` globs (glob-escaped) in the build-files hash; tests

## 2. Affected

- [ ] 2.1 Staleness: hash recompute and paranoid matchers in `affected`/`run` include the lock's included-build globs; tests
- [ ] 2.2 Compute: diffed file under an included-build root → everything with "build logic changed" reason (before outside-all-modules fallback); tests
- [ ] 2.3 Root rule: exclude `:` from everything-affected listings unless it declares source dirs; tests

## 3. Fixtures and integration tests

- [ ] 3.1 Fixture `tests/fixtures/composite-plugin-only`: `pluginManagement { includeBuild("gradle/conventions") }` (deliberately not named build-logic) with a precompiled convention plugin applied by a module; `tests/fixtures/composite-substituting`: module depends on `com.example:library` provided by `includeBuild("included-lib")`
- [ ] 3.2 Integration test (skips when no `gradle` on PATH): plugin-only fixture syncs to a normal lock with `included_builds = ["gradle/conventions"]`; substituting fixture keeps the refusal with a substitution reason

## 4. Docs and verification

- [ ] 4.1 README.md / README_RU.md: composite-build sentence reflects the new behavior
- [ ] 4.2 Quality gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green (integration tests exercised with a real Gradle)
- [ ] 4.3 Real-repo E2E on nowinandroid (record below): re-sync produces a normal lock recording `build-logic`; synthetic `core/model` change selects `:core:model` + reverse closure (not all 45); docs-only change exits 3 with `--quiet`; a `build-logic/` change triggers stale/paranoid handling and, with `--auto-sync`, the "build logic changed" everything reason; `:` absent from listings; working tree restored

## E2E record

(to be filled before archiving)
