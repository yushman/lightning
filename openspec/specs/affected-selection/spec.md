# affected-selection Specification

## Purpose
TBD - created by archiving change add-affected-selection. Update Purpose after archive.
## Requirements
### Requirement: Module graph sync
`lightning sync` SHALL run Gradle once (project wrapper preferred, `gradle` from PATH otherwise) with an embedded init script that captures the project model, and SHALL write `lightning.lock`: deterministic JSON (sorted modules, source dirs, tasks, edges) containing per module its path, root-relative directory, declared source-set directories (root-relative, including directories outside the module such as `srcDir("../shared")`), task names, and declared inter-module dependency edges typed `main` or `test`, plus a blake3 hash over the invalidation file set. Edges SHALL be typed `test` only when declared in `testImplementation`, `testApi`, `testCompileOnly`, or `testRuntimeOnly`; all other configurations SHALL yield `main` edges. When the build has included builds, sync SHALL detect dependency substitution by resolving the dependency graph of every resolvable configuration of every project in the root build and inspecting resolved project component identifiers for a build other than the root build (`BuildIdentifier.getBuildPath()` when available, `isCurrentBuild()` otherwise). A composite with no substituted module dependency and all included-build roots inside the Gradle root SHALL produce a normal lock recording the included builds' root-relative directories in `included_builds`. A composite where any module dependency resolves into an included build SHALL mark the lock unsupported with a reason naming the substituted components. When detection cannot run or verify (unavailable API, resolution error, included-build root outside the Gradle root), sync SHALL fail safe and mark the lock unsupported.

#### Scenario: Lock captures the model deterministically
- **WHEN** `lightning sync` runs twice on an unchanged multi-module build
- **THEN** both runs produce byte-identical `lightning.lock` files listing every module with its source dirs, tasks, and typed edges

#### Scenario: Plugin-only composite build supported
- **WHEN** `lightning sync` runs on a build whose only `includeBuild` provides convention plugins and no module dependency substitutes into it
- **THEN** the lock is written without an unsupported marker and `included_builds` records the included build's root-relative directory

#### Scenario: Substituting composite build marked unsupported
- **WHEN** `lightning sync` runs on a build where a module dependency resolves to a project of an included build
- **THEN** the lock is marked unsupported with a reason naming the substituted component

#### Scenario: Unverifiable substitution fails safe
- **WHEN** included builds exist and substitution detection cannot resolve a configuration's dependency graph
- **THEN** the lock is marked unsupported and affected selection degrades to everything

### Requirement: Lock invalidation
The invalidation hash SHALL cover, root-relatively and skipping `.git`/`.gradle`/`build` directories: `**/*.gradle`, `**/*.gradle.kts`, `buildSrc/**`, `build-logic/**`, `gradle/libs.versions.toml`, `gradle/wrapper/gradle-wrapper.properties`, `gradle.properties`, `local.properties`, `lightning.toml`, globs from `[affected] invalidate_on` in `lightning.toml`, plus `<dir>/**` for every directory recorded in the lock's `included_builds`. `lightning affected` and `lightning run` SHALL treat the lock as stale solely when the recomputed hash differs from the stored one; the diff content SHALL NOT by itself mark the lock stale. lightning's own files SHALL never map into the module graph: `lightning.lock` and `lightning.toml` SHALL be excluded from the diff (the config participates through the invalidation hash instead). A missing or stale lock SHALL abort with exit code 4 and a message pointing at `lightning sync`, unless `--auto-sync` is given, in which case sync runs first and selection proceeds. The lock format version SHALL be 2; a lock with any other version SHALL be rejected with an error instructing to run `lightning sync`.

#### Scenario: Stale lock refused
- **WHEN** a build file changes after the last sync and `lightning affected` runs without `--auto-sync`
- **THEN** it exits with code 4 and instructs the user to run `lightning sync`

#### Scenario: Auto-sync recovers
- **WHEN** the same situation occurs with `--auto-sync`
- **THEN** sync runs first and selection proceeds on the fresh lock

#### Scenario: Re-synced build-file change is not stale
- **WHEN** a build file is modified (committed or not) and `lightning sync` runs afterwards
- **THEN** a subsequent `lightning affected` proceeds without exit code 4, and the modified build file maps through the normal file-to-module rules

#### Scenario: Config change forces re-sync instead of everything-affected
- **WHEN** `lightning.toml` changes after the last sync
- **THEN** the lock is stale (hash mismatch); after re-sync, selection proceeds and `lightning.toml` never appears as a changed file mapping outside all modules

#### Scenario: Included-build root invalidates regardless of its name
- **WHEN** a file under an included build recorded in the lock (e.g. `gradle/plugins/**`) changes after the last sync
- **THEN** the lock is treated as stale (exit 4 without `--auto-sync`)

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
Each changed file SHALL map to modules by declared source-set directories first (all matching modules), then by longest module-directory prefix (the root project's directory SHALL NOT participate in prefix matching). A changed file inside a recorded included-build root SHALL make everything affected with a "build logic changed" reason naming the file and the included build. Any other changed file matching no module SHALL make everything affected: a warning names the file. A lock marked unsupported SHALL behave the same way. In every everything-affected degradation all modules are selected and `run` fans out to all selected modules that have the task, except the root project `:`, which SHALL be excluded from everything-affected listings (and `run` fan-out) unless it declares source dirs; selection of `:` through its own declared source dirs or edges is unaffected.

#### Scenario: Out-of-module source dir mapped
- **WHEN** a file under a directory declared via `srcDir("../shared")` changes
- **THEN** the declaring module is affected

#### Scenario: Unknown file degrades to everything
- **WHEN** a changed file lies outside every module directory and every declared source dir
- **THEN** all modules are reported affected with a warning

#### Scenario: Included-build file reported as build logic change
- **WHEN** the diff contains a file inside a recorded included-build root (after staleness handling, e.g. via `--auto-sync`)
- **THEN** everything is affected and the warning names the file and the included build as a build-logic change, not as a file outside all modules

#### Scenario: Bare root excluded from everything-affected output
- **WHEN** everything-affected fires on a build whose root project declares no source dirs
- **THEN** `:` is absent from the listed modules and receives no tasks from `run`

#### Scenario: Root with sources kept in everything-affected output
- **WHEN** everything-affected fires on a build whose root project declares source dirs
- **THEN** `:` is listed among the affected modules

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

