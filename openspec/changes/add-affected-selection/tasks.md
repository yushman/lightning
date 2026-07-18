## 1. Foundations

- [ ] 1.1 `lightning.toml` config module (`[affected] base/ignore/invalidate_on`, deny unknown keys, defaults on missing file); tests
- [ ] 1.2 Lock model (`lightning.lock` JSON: version, hash, unsupported, modules with dir/source_dirs/tasks/typed deps), deterministic serialization, load/save; tests
- [ ] 1.3 Invalidation hash: fixed glob set + config extras, blake3 over sorted path+content, skip `.git`/`.gradle`/`build`; tests

## 2. Sync

- [ ] 2.1 Groovy sync init script (separate asset, `include_str!`): settingsEvaluated composite detection, projectsEvaluated dump (root build only) of paths, dirs, source-set dirs, task names, declared project deps per configuration
- [ ] 2.2 `lightning sync`: locate gradle root, run wrapper/gradle with init script + dump property, normalize dump into sorted lock with hash; unit tests for normalization and edge typing (closed test-config list)

## 3. Affected

- [ ] 3.1 Git diff module: merge-base(base, HEAD) semantics, `--base`/`--base-sha`/config base, working-tree changes by default with `--no-uncommitted`, shallow-clone diagnostics, repo-root→lock-dir path relativization; tests where feasible
- [ ] 3.2 Mapping + closure: ignore globs, source-dir-first then longest-dir-prefix (root excluded), outside-file → everything, typed-edge closure (main transitive, test one hop into main-affected); unit tests
- [ ] 3.3 Property test: random DAGs + random diffs vs independent naive reference, superset invariants (hand-rolled deterministic RNG)
- [ ] 3.4 `lightning affected` command: staleness check (hash + paranoid glob) with exit 4 / `--auto-sync`, outputs text/`--json`/`--format github-matrix`/`--quiet` (exit 3), everything-affected degradations; tests

## 4. Run

- [ ] 4.1 `lightning run <task>`: affected → exact task-name match per module, skip notes, single Gradle invocation with `:module:task` paths + args after `--`, propagate exit code, nothing-affected fast path

## 5. Fixtures, docs, verification

- [ ] 5.1 Fixture Gradle project `tests/fixtures/multimodule`: core/lib/app main chain, fixtures module with test edges, `srcDir("../shared")`, `custom` module with registered `testDebugUnitTest` and no `test` task, docs dir + lightning.toml ignore
- [ ] 5.2 README: sync/affected/run usage, exit codes, lightning.toml, GHA matrix fan-out + fast-exit examples
- [ ] 5.3 Quality gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] 5.4 E2E with real Gradle (record below): (a) leaf-module change on a branch → exact closure incl. typed-edge behavior; (b) docs-only diff → `--quiet` exit 3; (c) build-file change → stale exit 4, `--auto-sync` recovers; (d) `run test` invokes Gradle for affected modules only, skips module lacking the task; (e) file outside modules → everything affected; (f) `--format github-matrix` is valid JSON
- [ ] 5.5 Phase 1–3 regression: workspace tests green, server smoke (`/`, `/builds`, `/trends`, `/cache`, ingest)

## E2E record

(to be filled during verification)
