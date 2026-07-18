## MODIFIED Requirements

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
