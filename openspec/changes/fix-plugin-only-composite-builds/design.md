## Context

`lightning affected` derives selection purely from `lightning.lock`. The lock is trustworthy only if the recorded module graph is the whole graph. Included builds were refused wholesale because dependency substitution can route a module dependency into another build the lock does not model. But the flagship layout — `includeBuild("build-logic")` for convention plugins — never substitutes anything: the included build only contributes to the plugin classpath. Refusing it makes phase 4 useless on exactly the projects it targets. Guiding invariant unchanged: **false negatives never** — every ambiguity degrades to everything-affected or a refusal.

## Goals / Non-Goals

**Goals:**
- Support plugin-only composite builds: normal lock, normal selection.
- Keep the honest refusal for composites where any module dependency substitutes into an included build.
- Close the invalidation hole: changes under any included-build root (whatever its name) must invalidate the lock.
- Stop listing the root project `:` in everything-affected output when it carries no sources.

**Non-Goals:**
- Modeling cross-build dependency graphs (substituting composites stay refused).
- Watching included builds located outside the Gradle root (refused instead).

## Decisions

### D12. Substitution detection via resolved ProjectComponentIdentifier

During sync, when `gradle.includedBuilds` is non-empty, the init script resolves the dependency graph (graph metadata only, no artifact downloads) of every resolvable configuration of every project in the root build and inspects each resolved component id. A `ProjectComponentIdentifier` whose build is not the root build proves dependency substitution into an included build. Build identity uses public API across Gradle 7/8/9: `BuildIdentifier.getBuildPath() == ':'` when available (Gradle 8.2+), else `BuildIdentifier.isCurrentBuild()` (pre-9; the script runs in the root build, so current == root). If neither works, the build is treated as foreign — fail safe.

- No substitution found → the composite is plugin-only: `unsupported` stays null and the lock records the included builds' root-relative dirs.
- Any substitution → `unsupported: "dependency substitution into included build(s): ..."` naming the foreign components; today's everything-affected refusal is preserved.
- A configuration whose graph resolution throws → `unsupported: "cannot verify dependency substitution (...)"` — fail safe, refusal kept.
- An included-build root outside the Gradle root (`../...`) cannot be covered by the invalidation walk → refusal with an explicit reason.

FN-never argument: substitution rewrites a dependency selector *before* resolution, and a substituted-to-project edge always resolves locally, so an `UnresolvedDependencyResult` (e.g. offline metadata miss) can never hide a substitution — unresolved dependencies are safely ignored; only a thrown resolution error is unverifiable and triggers the refusal. Buildscript/plugin classpaths are deliberately not scanned: plugin substitution is exactly the plugin-only case, and its effects on module configuration are captured in the lock plus guarded by D13 invalidation. Configurations with no declared dependencies are skipped (their graph is trivially empty); transitive substitution is covered because whole graphs of configurations with any dependency are resolved. Detection only ever errs towards refusal.

Cost: graph resolution runs only when included builds exist, only at sync time (the hot path is untouched). Cached metadata makes it seconds-to-a-minute on real projects.

### D13. Lock v2 and dynamic invalidation

`lightning.lock` version bumps to 2 and gains `included_builds: [<root-relative dir>...]` (sorted, normalized; empty for non-composites). The existing version check rejects v1 locks with "run `lightning sync`" — old locks are stale, never misparsed. Each recorded dir contributes a `<dir>/**` glob (glob-escaped) to the invalidation set used for (a) the build-files hash at sync time, (b) the hash recompute in `affected`/`run`, and (c) the paranoid diff check — symmetric by construction because both sides read the same lock field. The static `build-logic/**` glob stays for back-compat but is no longer load-bearing. Dirs recorded with leading `..` never occur (D12 refuses them) but would be harmless: the hash walk cannot reach them on either side.

### D14. Included-build files degrade with an honest reason

Previously a diffed file under `build-logic/` hit the paranoid staleness check (exit 4 or auto-sync) and then — after auto-sync — fell through file→module mapping to "changed file X is outside all modules → everything". The set was right, the reason misleading. Now `compute` checks `lock.included_builds` first: a file under an included-build root degrades to everything with reason "build logic changed: <file> is inside included build <dir>". A convention-plugin change can reconfigure any module, so everything is the only safe answer; the message now says why. Files under module dirs (e.g. `app/build.gradle`) keep mapping to their module (D6), and the stale/paranoid gate still fires before selection.

### D15. Root module listing rule

The root project `:` appears in output only when it earned selection through the graph (its declared source dirs or edges — unchanged, D6). In everything-affected degradations `:` is excluded from the listed modules (and from `run` fan-out) **unless it declares source dirs**: a typical multi-module root is a plugin-application container with no code, and blanket-listing it schedules meaningless root tasks; a root that genuinely declares sources may carry tests, so FN-never demands it stays included there. Non-root modules are never filtered.

## Risks / Trade-offs

- [Resolution at sync time can be slow or throw on exotic configurations] → runs only for composites; any throw keeps the refusal (over-selection, never FN). Verified on nowinandroid.
- [Custom `dependencySubstitution` rules in settings] → they manifest as resolved foreign `ProjectComponentIdentifier`s exactly like automatic substitution, so detection catches them.
- [Included build touched in a diff always re-syncs under `--auto-sync`] → same behavior `build-logic/**` already had; correctness over speed.
- [v1 → v2 lock migration] → one explicit re-sync, guided by the error message.

## Migration Plan

Re-run `lightning sync` once per repo (v1 locks are rejected with that exact instruction). No config or CLI surface changes.

## Open Questions

None.
