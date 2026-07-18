## Context

Phase 4 of the lightning platform (`docs/design/lightning-platform.md` §5 fixes the grooming decisions): selective execution outside the build. A hybrid design — `lightning sync` pays Gradle configuration cost once to snapshot the module graph into `lightning.lock`; the hot path (`affected`, `run`) is pure Rust over git diff and the lock, so fan-out decisions and docs-only fast-exits happen before any JVM starts. Guiding principle #2: **false negatives never** — a skipped test is unacceptable, an extra run is fine; every ambiguity degrades to "everything affected".

## Goals / Non-Goals

**Goals:**
- `sync` → deterministic `lightning.lock` (modules, source sets, tasks, typed edges, invalidation hash).
- `affected` with text / json / github-matrix / exit-code outputs, merge-base semantics, working-tree inclusion.
- `run <task>` sugar mapping the task per module and delegating to Gradle.
- Property-tested FN-never closure.

**Non-Goals:**
- Android variant awareness (an Android module is a module like any other; task names come from the lock).
- Composite/included builds (detected, honest refusal: everything affected).
- File-level (intra-module) selection; parsing build files without Gradle; server involvement.

## Decisions

### D1. Lock model and determinism

`lightning.lock` is pretty-printed JSON: `{version, build_files_hash, unsupported (optional reason string), modules[]}`; each module has `path` (`:app`), `dir` (root-relative, `"."` for root), `source_dirs` (root-relative, normalized, may leave the module dir — this is what catches `srcDir("../shared")`), `tasks` (names only), `deps` (`{path, kind: main|test}`). The init script dumps a raw model; the **CLI** sorts modules by path, and sorts+dedups source dirs, tasks, and deps, so the lock is diff-friendly and independent of Gradle iteration order. Only declared `ProjectDependency`s are recorded (resolution is not needed and would be slow).

### D2. Sync mechanics

`sync` must run in the Gradle root directory (checked: `settings.gradle(.kts)` or `build.gradle(.kts)` present). It writes the embedded Groovy init script (`include_str!`, separate asset from telemetry) to a temp file and runs `./gradlew` (or `gradle` when no wrapper) with `--init-script <tmp> -Plightning.lock.dump=<tmp-out> help -q`. The script hooks `settingsEvaluated` (to detect included builds) and `projectsEvaluated` (root build only, `gradle.parent == null`), collecting per project: path, dir, `sourceSets[*].allSource.srcDirs` when the container exists (relativized to the root project dir), `tasks.names` (no task realization), and declared project dependencies per configuration. Included builds present → the dump carries `unsupported: "composite build (included builds: ...)"`; the lock stores it and `affected` degrades (D8).

### D3. Edge typing: closed test list, unknown → main

An edge's kind comes from the configuration it is declared in. `test` iff the configuration name is exactly one of `testImplementation`, `testApi`, `testCompileOnly`, `testRuntimeOnly`. **Everything else is `main`** — including `testFixturesImplementation`, `androidTestImplementation`, custom suites, `kapt`/`ksp`, etc. Rationale (FN-never): typing an edge `test` narrows propagation, so it is only safe for configurations that provably feed nothing consumable to other modules; the default-JVM unit-test configurations are exactly that set. Misclassifying towards `main` merely over-selects. (Counter-example that forbids a name-contains-"test" heuristic: `x` declares `testFixturesImplementation project(":y")`; `z`'s tests consume `testFixtures(x)`; a change in `y` must affect `z`, which requires the `x→y` fixtures edge to propagate like `main`.) Duplicate edges to the same module keep both kinds; `main` wins during traversal by construction.

### D4. Invalidation hash

blake3 over the sorted list of matching files, each contributing `path NUL len NUL bytes`, matched root-relatively: `**/*.gradle`, `**/*.gradle.kts` (covers `settings.*` and per-module build files), `buildSrc/**`, `build-logic/**`, `gradle/libs.versions.toml`, `gradle/wrapper/gradle-wrapper.properties`, `gradle.properties`, `local.properties`, plus user globs from `[affected] invalidate_on`. The walk skips `.git`, `.gradle`, and `build` directories (generated files must not flap the hash). Staleness in `affected`/`run`: (a) recomputed working-tree hash ≠ lock hash, or (b) **paranoid mode** — the diff itself touches any invalidation glob (guards CI restoring a lock from a mismatched cache key). Stale → exit 4 with a "run lightning sync" message, unless `--auto-sync`, which runs sync in-process and proceeds (paranoid is satisfied by construction after a fresh sync). Missing lock is reported the same way (exit 4).

### D5. Base and diff semantics

Changed files = `git diff --name-only <merge-base(base, HEAD)>..HEAD` (three-dot semantics via explicit merge-base) ∪ working-tree changes (`git status --porcelain`, staged/unstaged/untracked) unless `--no-uncommitted`. Base defaults to `origin/main`, overridable by `[affected] base` and `--base <ref>`; `--base-sha <sha>` skips merge-base entirely (escape hatch for CI that already knows the exact sha). Merge-base failure checks `git rev-parse --is-shallow-repository` and, when shallow, fails with an actionable fetch-depth/`--unshallow` message; an unknown base ref gets a "fetch the base ref" message. Paths are repo-root-relative (`--show-toplevel`) and re-relativized to the lock directory; a diff path above the lock directory is "outside all modules" (D6). Ignore globs (`[affected] ignore`, **no defaults**) are applied to the diff before anything else — opt-in because any default (e.g. `**/*.md`) is a silent FN risk: a README can be a fixture or an input to codegen; only the user can certify a path as build-irrelevant. False positives from not ignoring are just wasted compute.

### D6. File→module mapping

Per changed file: (1) modules whose declared source dirs contain the file — all matches win (overlaps select both, safe); (2) otherwise the module with the longest `dir` prefix, **excluding the root project's `"."`** — with root included nothing would ever be "outside", defeating the safe fallback; the root module is reachable via its declared source dirs only; (3) otherwise the file is outside all modules → **everything affected**, with a stderr warning naming the first such file. Build files that survive the ignore filter and paranoid re-sync map like any other file (e.g. `app/build.gradle` → `:app`); root-level build files fall outside → everything. Deleted files map by path string, same rules.

### D7. Closure formula (typed edges)

Reverse-reachability with one-hop test edges. Let `changed` = modules containing changed files. `main_affected` = `changed` ∪ every module from which some changed module is reachable via **main edges only** (transitively). `affected` = `main_affected` ∪ every module with a **direct test edge into `main_affected`**. Test edges do not propagate: a module affected only via its test edge does not make its own dependents affected. Note the deliberate widening over the literal grooming phrasing ("changed module directly reachable via test-edges"): the test hop lands on `main_affected`, not just `changed` — if `m —test→ t —main→ c(changed)`, `m`'s tests link `t`'s output which embeds `c`, so `m` must be selected; restricting the hop to literally-changed modules would be a false negative. Implementation: reverse-BFS over main edges from all changed modules, then one sweep over test edges. The property test (D10) pins this formula against an independent naive implementation.

### D8. Everything-affected degradations

Uniform behavior when selection cannot be trusted: file outside all modules; lock marked `unsupported` (composite build). Output modes then list **all** modules (text/json/matrix, json carries `everything: true` + reason), `--quiet` exits 0, and `run` fans out to every module that has the task. Missing/stale lock deliberately does **not** degrade — it aborts with exit 4, because "everything affected" there would mask a broken setup forever; `--auto-sync` is the CI-friendly path.

### D9. CLI contract

- `lightning sync [--out lightning.lock]` — exit 0/1.
- `lightning affected [--base <ref> | --base-sha <sha>] [--auto-sync] [--no-uncommitted] [--json | --format text|json|github-matrix] [--quiet]`.
- `lightning run <task> [same selection flags] [-- <extra gradle args>]` — skips affected modules lacking the task (stderr note), invokes one Gradle build with all `:module:task` paths, exit code = Gradle's; nothing affected → message, exit 0, no Gradle.
- Formats: text = one module path per line (nothing affected → empty). json = `{base, merge_base, everything, reason, modules: [{path, reason: changed|main-dependency|test-dependency}]}`. github-matrix = `{"include": [{"module": ":app"}, ...]}` for `strategy.matrix: ${{ fromJSON(...) }}`.
- Exit codes (documented in README): 0 success (in `--quiet`: something affected), 1 error, 2 CLI usage (clap), 3 `--quiet` and nothing affected, 4 missing/stale lock.

### D10. Property test (FN-never)

Hand-rolled deterministic generator (xorshift, no new dev-dependency): hundreds of random DAGs (edges only from higher to lower index → acyclic by construction, random main/test kinds, random fan-out) with random changed sets. The optimized closure must (a) equal an **independently coded naive reference** — per module, DFS "can I reach a changed module via main edges", then a separate literal test-edge sweep — and (b) satisfy the superset invariants `affected ⊇ changed` and `affected ⊇ main_affected`. Fixture Gradle projects under `tests/fixtures/multimodule` (plain JVM: main/test edge mix, `srcDir("../shared")`, a module with a registered `testDebugUnitTest` task and no `test` task simulating Android-style naming divergence — no AGP in fixtures) serve the real-Gradle E2E.

### D11. Config format

`lightning.toml` in the working directory, parsed with the `toml` crate, only:

```toml
[affected]
base = "origin/main"          # default base ref
ignore = ["docs/**"]          # opt-in, no defaults (D5)
invalidate_on = ["deps.lock"] # extra invalidation globs (D4)
```

Missing file → defaults. Unknown keys are rejected (`deny_unknown_fields`) so typos fail loudly instead of silently weakening selection.

## Risks / Trade-offs

- [Lock can lag reality between syncs] → the invalidation hash covers every file that can change the graph; paranoid mode additionally forces re-sync when the diff touches those files.
- [Module build-file change maps only to its module] → cross-project configuration from a subproject's build script (`rootProject.allprojects {...}`) would under-select; accepted and documented — root-level build files and `buildSrc`/`build-logic` (the sanctioned cross-cutting mechanisms) already degrade to everything.
- [`gradle help` must configure all projects] → true with default settings; configure-on-demand builds are out of scope (documented), and the hash set includes `gradle.properties` where it would be enabled.
- [Working-tree inclusion makes local runs noisy] → intended default (matches "what will CI see if I push"); `--no-uncommitted` for scripted use.
- [Source-dir overlap selects multiple modules] → over-selection only, consistent with FN-never.

## Migration Plan

Purely additive CLI surface; no server, schema, or existing-command changes. New files: `lightning.lock` (recommended: CI-cache keyed by the build-files hash; committing is opt-in), `lightning.toml` (optional).

## Open Questions

None blocking. Deferred: variant-aware Android selection, included-build support, feeding the phase-2 telemetry graph back into the lock.
