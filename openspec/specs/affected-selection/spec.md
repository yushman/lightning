# affected-selection Specification

## Purpose
TBD - created by archiving change add-affected-selection. Update Purpose after archive.
## Requirements
### Requirement: Module graph sync
`lightning sync` SHALL run Gradle once (project wrapper preferred, `gradle` from PATH otherwise) with an embedded init script that captures the project model, and SHALL write `lightning.lock`: deterministic JSON (sorted modules, source dirs, tasks, edges) containing per module its path, root-relative directory, declared source-set directories (root-relative, including directories outside the module such as `srcDir("../shared")`), task names, and declared inter-module dependency edges typed `main` or `test`, plus a blake3 hash over the invalidation file set. Edges SHALL be typed `test` only when declared in `testImplementation`, `testApi`, `testCompileOnly`, or `testRuntimeOnly`; all other configurations SHALL yield `main` edges. When the build has included builds, the lock SHALL be marked unsupported with a reason.

#### Scenario: Lock captures the model deterministically
- **WHEN** `lightning sync` runs twice on an unchanged multi-module build
- **THEN** both runs produce byte-identical `lightning.lock` files listing every module with its source dirs, tasks, and typed edges

#### Scenario: Composite build marked unsupported
- **WHEN** `lightning sync` runs on a build with `includeBuild`
- **THEN** the lock is written with an unsupported reason naming the included builds

### Requirement: Lock invalidation
The invalidation hash SHALL cover, root-relatively and skipping `.git`/`.gradle`/`build` directories: `**/*.gradle`, `**/*.gradle.kts`, `buildSrc/**`, `build-logic/**`, `gradle/libs.versions.toml`, `gradle/wrapper/gradle-wrapper.properties`, `gradle.properties`, `local.properties`, plus globs from `[affected] invalidate_on` in `lightning.toml`. `lightning affected` and `lightning run` SHALL treat the lock as stale when the recomputed hash differs from the stored one, or when the diff itself touches any invalidation glob. A missing or stale lock SHALL abort with exit code 4 and a message pointing at `lightning sync`, unless `--auto-sync` is given, in which case sync runs first and selection proceeds.

#### Scenario: Stale lock refused
- **WHEN** a build file changes after the last sync and `lightning affected` runs without `--auto-sync`
- **THEN** it exits with code 4 and instructs the user to run `lightning sync`

#### Scenario: Auto-sync recovers
- **WHEN** the same situation occurs with `--auto-sync`
- **THEN** sync runs, the lock is refreshed, and the affected set is computed and printed

### Requirement: Diff and base semantics
Changed files SHALL be computed as `git diff --name-only` against `merge-base(base, HEAD)` plus, by default, uncommitted working-tree changes including untracked files (`--no-uncommitted` disables the latter). The base ref SHALL default to `origin/main`, overridable by `[affected] base` in `lightning.toml` and by `--base <ref>`; `--base-sha <sha>` SHALL diff directly against the given commit without merge-base resolution. When merge-base resolution fails in a shallow clone, the error SHALL name the shallow clone and suggest fetching history (e.g. `fetch-depth: 0`). Files matching `[affected] ignore` globs (opt-in, no defaults) SHALL be excluded from the diff before mapping.

#### Scenario: Merge-base diff
- **WHEN** a branch diverged from `origin/main` and both sides gained commits
- **THEN** only the branch's own changes count, not changes merged to `origin/main` since divergence

#### Scenario: Ignored files excluded
- **WHEN** the diff contains only files matching configured ignore globs
- **THEN** nothing is affected

#### Scenario: Shallow clone diagnosed
- **WHEN** merge-base fails because the repository is shallow
- **THEN** the command fails with an actionable message about fetch depth

### Requirement: File-to-module mapping with safe fallback
Each changed file SHALL map to modules by declared source-set directories first (all matching modules), then by longest module-directory prefix (the root project's directory SHALL NOT participate in prefix matching). A changed file matching no module SHALL make everything affected: all modules are selected, a warning names the file, and `run` fans out to all modules that have the task. A lock marked unsupported (composite build) SHALL behave the same way.

#### Scenario: Out-of-module source dir mapped
- **WHEN** a file under a directory declared via `srcDir("../shared")` changes
- **THEN** the declaring module is affected

#### Scenario: Unknown file degrades to everything
- **WHEN** a changed file lies outside every module directory and every declared source dir
- **THEN** all modules are reported affected with a warning

### Requirement: Typed-edge affected closure
A module SHALL be affected iff it contains changed files, or a changed module is reachable from it via `main` edges transitively, or one of its direct `test` edges points at a module in that main-affected set. Test edges SHALL NOT propagate further: a module affected only via its own test edge does not affect its dependents. The closure implementation SHALL be verified by a property test on random DAGs and random change sets against an independent naive reference implementation, including the invariant that the affected set is a superset of the changed and main-affected sets.

#### Scenario: Main edges propagate transitively
- **WHEN** `:app` depends (main) on `:lib` which depends (main) on changed `:core`
- **THEN** `:core`, `:lib`, and `:app` are affected

#### Scenario: Test edges do not propagate
- **WHEN** `:app` has a test edge to changed `:fixtures`, and `:other` depends (main) on `:app`
- **THEN** `:app` is affected but `:other` is not

### Requirement: Affected outputs and exit codes
`lightning affected` SHALL print affected module paths one per line by default; `--json` (or `--format json`) SHALL emit a JSON object with the base, merge-base sha, an `everything` flag with reason, and per-module reasons (`changed`, `main-dependency`, `test-dependency`); `--format github-matrix` SHALL emit `{"include": [{"module": ":path"}...]}` valid as a GitHub Actions matrix. `--quiet` SHALL print nothing and exit 0 when something is affected and 3 when nothing is. Exit code 4 SHALL be reserved for missing/stale lock, 1 for other errors.

#### Scenario: Quiet fast-exit
- **WHEN** the diff maps to no modules and `--quiet` is given
- **THEN** the command exits with code 3 and no output

#### Scenario: GitHub matrix output
- **WHEN** `--format github-matrix` is given and modules are affected
- **THEN** stdout is a single valid JSON object with one `include` entry per affected module

### Requirement: Selective task execution
`lightning run <task>` SHALL compute the affected set (same flags as `affected`), select affected modules whose lock task list contains the task name exactly (modules lacking it are skipped with a stderr note), and invoke Gradle once with the resulting `:module:task` paths plus any arguments after `--`, propagating Gradle's exit code. When nothing is affected, it SHALL print a message and exit 0 without invoking Gradle.

#### Scenario: Task fan-out over affected modules
- **WHEN** `:core` and `:custom` are affected, `:core` has a `test` task and `:custom` does not, and `lightning run test` executes
- **THEN** Gradle is invoked with `:core:test` only and `:custom` is reported as skipped

#### Scenario: Nothing affected skips Gradle
- **WHEN** the affected set is empty and `lightning run test` executes
- **THEN** no Gradle process starts and the exit code is 0

### Requirement: Configuration file
The CLI SHALL read an optional `lightning.toml` from the working directory with only the `[affected]` table and keys `base` (string), `ignore` (string array), `invalidate_on` (string array). Unknown keys SHALL be rejected with an error. A missing file SHALL mean defaults (base `origin/main`, no ignore globs, no extra invalidation globs).

#### Scenario: Config overrides base
- **WHEN** `lightning.toml` sets `base = "origin/develop"` and no `--base` flag is given
- **THEN** the diff is computed against `merge-base(origin/develop, HEAD)`

#### Scenario: Typo rejected
- **WHEN** `lightning.toml` contains an unknown key under `[affected]`
- **THEN** the command fails with a parse error naming the key

