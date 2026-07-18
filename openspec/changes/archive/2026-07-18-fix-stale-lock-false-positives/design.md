# fix-stale-lock-false-positives — design

## Context

Staleness in `affected.rs::select` has two branches: (1) recomputed blake3 hash over the invalidation set differs from `lock.build_files_hash`; (2) otherwise, any diff file matching an invalidation matcher is reported as "the diff touches build file X". Branch (2) came from the original grooming as a "paranoid CI mode" for build-file-touching diffs. At that time it was conceived as a safety net; implementation reality made it redundant.

## Decision: hash is the sole staleness authority

The hash is recomputed on **every** `affected`/`run` invocation over the **current working tree**, covering the complete invalidation set: static globs (`**/*.gradle(.kts)`, `buildSrc/**`, `build-logic/**`, catalog, wrapper, properties), user `invalidate_on`, and dynamic `<included-build>/**` globs from the lock. File list and contents both feed the hash.

FN-never argument for removing branch (2): for the lock to be wrongly considered fresh, a file influencing the module graph must differ from its state at sync while the hash stays equal. The hash covers every file the graph is derived from (same set sync hashes); equal hash ⇒ byte-equal invalidation set ⇒ the graph snapshot corresponds to the current build configuration. The diff-vs-git-base comparison is irrelevant to that implication — git base is not the sync baseline. All cases branch (2) catches with a *content difference from sync state* are already caught by branch (1); the remainder are exactly the false positives observed (lock synced after the change; extracted init script present at sync time).

Behavioral consequence: a build file changed in a PR *without* re-sync still exits 4 via the hash (CI lock from cache keyed on these same files misses → sync). A build file changed *and then synced* proceeds — which is the correct and previously-broken case.

## Out of scope

- Excluding lightning's own artifacts by name (unnecessary once hash rules; extracting the init script into the repo is one legitimate re-sync, and the README now suggests a temp path).
- Any lock format or sync change.
