## MODIFIED Requirements

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
The invalidation hash SHALL cover, root-relatively and skipping `.git`/`.gradle`/`build` directories: `**/*.gradle`, `**/*.gradle.kts`, `buildSrc/**`, `build-logic/**`, `gradle/libs.versions.toml`, `gradle/wrapper/gradle-wrapper.properties`, `gradle.properties`, `local.properties`, globs from `[affected] invalidate_on` in `lightning.toml`, plus `<dir>/**` for every directory recorded in the lock's `included_builds`. `lightning affected` and `lightning run` SHALL treat the lock as stale when the recomputed hash differs from the stored one, or when the diff itself touches any invalidation glob (including the dynamic included-build globs). A missing or stale lock SHALL abort with exit code 4 and a message pointing at `lightning sync`, unless `--auto-sync` is given, in which case sync runs first and selection proceeds. The lock format version SHALL be 2; a lock with any other version SHALL be rejected with an error instructing to run `lightning sync`.

#### Scenario: Stale lock refused
- **WHEN** a build file changes after the last sync and `lightning affected` runs without `--auto-sync`
- **THEN** it exits with code 4 and instructs the user to run `lightning sync`

#### Scenario: Auto-sync recovers
- **WHEN** the same situation occurs with `--auto-sync`
- **THEN** sync runs, the lock is refreshed, and the affected set is computed and printed

#### Scenario: Included-build root invalidates regardless of its name
- **WHEN** the lock records included build `gradle/plugins` and a file under `gradle/plugins/` changes
- **THEN** the lock is treated as stale (exit 4 without `--auto-sync`)

#### Scenario: Old lock version rejected
- **WHEN** `lightning affected` loads a version-1 lock
- **THEN** it fails with an error naming the version mismatch and suggesting `lightning sync`

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
