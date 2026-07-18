## Why

Dogfooding on nowinandroid (the dominant modern Android layout) showed that `lightning sync` marks ANY composite build as unsupported. nowinandroid uses `includeBuild("build-logic")` purely for convention plugins, yet `affected` degrades to everything-affected (all 45 modules), making selective execution worthless exactly where it matters most. A plugin-only included build is safe to support: no module dependency resolves into it, so the module graph in the lock is complete; the only risk — build-logic changes reconfiguring modules — is already the lock invalidation problem. Additionally, the root project `:` is cosmetically listed as affected in everything-affected mode even when it carries no sources.

## What Changes

- Sync init script: when included builds exist, detect whether any module dependency resolves to a project in another build (dependency substitution) via resolved `ProjectComponentIdentifier`s. Plugin-only composites (no substitution, all included-build roots inside the Gradle root) produce a normal lock that records the included builds' root directories; substituting composites keep today's honest refusal. Detection that cannot run or verify fails safe (refusal).
- Lock schema v2: new `included_builds` field (sorted root-relative dirs). Old v1 locks are rejected with a clear re-sync error (existing version check).
- Dynamic invalidation: each recorded included-build root joins the invalidation set as `<root>/**` for both the build-files hash and the paranoid diff check, so e.g. `gradle/plugins/**` invalidates the lock even though it is not named `build-logic` (the static `build-logic/**` glob stays for back-compat). A diffed file inside an included-build root degrades to everything with an explicit "build logic changed" reason instead of the misleading "file outside all modules".
- Root module rule: `:` is excluded from everything-affected listings (and thus `run` fan-out) unless it declares source dirs; closure-based selection of `:` via its own source dirs is unchanged.
- Fixtures: `tests/fixtures/composite-plugin-only` (convention plugin from `gradle/conventions`, deliberately not named build-logic) and `tests/fixtures/composite-substituting` (module depends on a library an included build provides), plus a gated integration test asserting plugin-only syncs to a normal lock while substituting keeps the refusal.
- README.md / README_RU.md: composite-build sentence updated (plugin-only included builds supported; substitution still degrades).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `affected-selection`: composite-build handling in sync (plugin-only supported, substitution refused), lock schema v2 with `included_builds`, dynamic invalidation globs, "build logic changed" degradation reason, root-module listing rule.

## Impact

- CLI: `lightning.sync.init.gradle` (substitution detection), `lock.rs` (VERSION 2, `included_builds`), `sync.rs` (dump parsing, hash globs), `affected.rs` (staleness globs, compute reasons, root exclusion).
- Repo: new composite fixtures; README updates. Existing v1 locks become stale (explicit error suggesting `lightning sync`).
- Server: no changes.
