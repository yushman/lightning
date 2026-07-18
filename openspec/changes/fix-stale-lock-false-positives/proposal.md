# fix-stale-lock-false-positives

## Why

Dogfooding on nowinandroid surfaced a false-stale loop: with an uncommitted (or committed-in-PR) build-file change, `lightning affected` reports the lock stale even immediately after `lightning sync`, because staleness is judged by two signals — the recomputed invalidation hash AND a diff-based check ("the diff touches a build file"). The diff-based check fires whenever a build file differs from the git base, regardless of whether the lock already reflects that content. Consequences observed on a real repo: a modified `settings.gradle.kts` (or our own extracted `lightning.init.gradle` sitting in the repo root, as the telemetry README suggests) makes `affected` permanently exit 4, and `--auto-sync` re-runs Gradle on every single invocation.

The diff-based check is strictly redundant: the hash is computed over the full invalidation set (static globs, user `invalidate_on`, dynamic included-build roots) against the current working tree on every invocation. Any build-file content or file-list change since the last sync flips the hash. The diff check therefore adds no false-negative protection — only false positives.

## What Changes

- `lightning affected` and `lightning run` treat the lock as stale **solely** on hash mismatch (or missing lock / wrong version). The "diff touches an invalidation glob" check is removed.
- `lightning.toml` joins the invalidation hash set (its content shapes selection semantics, so a change forces re-sync) and is excluded from the diff like `lightning.lock` — dogfooding showed an uncommitted config permanently degrading selection to everything-affected via "outside all modules".
- Regression test: modify a build file, re-sync, run `affected` — selection proceeds (no stale), and the modified build file maps through normal file-to-module rules.
- README (EN/RU): recommend extracting the telemetry init script outside the repo (e.g. a temp dir) to avoid one forced re-sync; no behavioral dependency remains either way.

## Capabilities

### Modified Capabilities

- `affected-selection`: staleness definition narrowed to hash mismatch only.

## Impact

- crates/cli: `affected.rs` (staleness branch), tests.
- No lock format change, no server change. Strictly fewer exit-4 outcomes; FN-never unaffected (hash remains authoritative and complete).
