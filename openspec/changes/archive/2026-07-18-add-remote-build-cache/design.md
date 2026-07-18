## Context

Phase 3 of the lightning platform (`docs/design/lightning-platform.md`): remote Gradle build cache in the existing single-binary server, plus cache analytics over phase-2 telemetry. Established patterns apply: rusqlite behind a `Mutex`, config via clap flags + env vars, server-rendered HTML, retention pruning at startup and on ingest.

## Goals / Non-Goals

**Goals:**
- Gradle HTTP build cache protocol, drop-in for `remote(HttpBuildCache)` in `settings.gradle`.
- Bounded storage: per-artifact size limit, total size cap with LRU eviction, last-access TTL.
- Optional shared-token write protection using Gradle's native Basic-auth credentials.
- Cache analytics: storage stats, overall/per-build hit rate, never-cached task paths.

**Non-Goals:**
- Cache entry inspection/decoding (entries are opaque blobs; Gradle's format is internal).
- Multi-node replication, S3/object-storage backends.
- Per-user accounts or scoped tokens; read auth.
- Correlating individual cache GETs/PUTs with specific builds (no build identity in the protocol).

## Decisions

### D1. Protocol surface

`GET /cache/{key}` → `200` with the artifact bytes as `application/octet-stream`, or `404` when absent. `PUT /cache/{key}` → stores the opaque body, `201`; re-PUT of an existing key overwrites (Gradle may race two builds pushing the same key — last write wins, both bodies are valid for that key). `HEAD /cache/{key}` → same status as GET without a body (axum serves HEAD through the GET route; hits are not counted for HEAD). Gradle appends the key directly to the configured URL, so the settings URL must end with `/cache/`.

### D2. Key validation: lowercase hex, 32–64 chars

Keys are validated as lowercase hex with length 32 to 64; anything else → `400`. The phase brief assumed 64-char SHA-256 keys, but empirically Gradle 9.6.1 produces 32-char (128-bit) keys — a strict 64-char rule would reject every real request. The 32–64 range covers current Gradle and a future hash widening without accepting arbitrary path input. Validated keys are safe filenames by construction.

### D3. Storage layout: flat files + SQLite index

Artifacts live as flat files `<cache-dir>/<key>`; the default cache dir is `lightning-cache` next to the SQLite db file (`--cache-dir` / `LIGHTNING_CACHE_DIR` to override). Writes go to a temp file (`.tmp-<pid>-<seq>`) in the same directory, then rename — readers never observe partial artifacts. Metadata is indexed in SQLite:

```sql
cache_entries(key TEXT PRIMARY KEY, size INTEGER NOT NULL,
              created_at TEXT NOT NULL DEFAULT (datetime('now')),
              last_accessed_at TEXT NOT NULL DEFAULT (datetime('now')),
              hit_count INTEGER NOT NULL DEFAULT 0)
```

The index is authoritative. Startup reconciles both directions: index rows without a file are dropped, files without an index row (and leftover temp files) are deleted. A GET whose file vanished degrades to a miss and drops the row. At ~1 MiB average artifacts and a 10 GiB cap, a flat directory holds ~10k files — fine on any modern filesystem.

### D4. Size limits

Max artifact size: 100 MiB default (`--cache-max-artifact-mb`), same threshold Gradle's own remote cache client warns/rejects at; oversized PUTs get `413` (enforced via the request body limit on the cache route). Max total cache size: 10 GiB default, configured in MiB (`--cache-max-size-mb`, default 10240) so tests and small installs can set tiny caps.

### D5. Eviction: LRU on write + last-access TTL

After each successful PUT, while the indexed total exceeds the cap, the least-recently-accessed entries (oldest `last_accessed_at`, excluding the key just written) are evicted — rows deleted, then files. GET updates `last_accessed_at` and `hit_count`, so hot entries survive. Entries not accessed for `--cache-retention-days` (default 30) are pruned at startup and on writes; TTL runs on last access, not creation, matching LRU semantics (a hot old entry is still valuable). Eviction decisions happen under the db mutex; file deletion after, outside the lock.

### D6. Optional write auth: shared token as Basic password

If `LIGHTNING_CACHE_TOKEN` (or `--cache-token`) is set, `PUT` requires `Authorization: Basic` where the password equals the token; the username is ignored. This is exactly what Gradle supports natively (`remote(HttpBuildCache) { credentials { username; password } }`). Missing/wrong credentials → `401` with `WWW-Authenticate: Basic`. Unset token → open writes. Reads are always open: cache artifacts are derived build outputs, and the threat model here is cache poisoning, not confidentiality. One new dependency, `base64`, to parse the header.

### D7. Analytics: hit rate and never-cached heuristic

- **Overall hit rate** (on `/cache`, from phase-2 `task_executions`): `from-cache / (from-cache + success + failed)` — the share of tasks that needed work but were restored from cache. `up-to-date` and `skipped` tasks are excluded from the denominator (no work was needed). The denominator includes non-cacheable tasks, so the number understates the achievable rate — stated on the page.
- **Per-build hit rate** (build detail): same formula over one build's tasks.
- **Never-cached task paths** (on `/cache`): over the last 100 builds, task paths executed (`success`/`failed`) at least 3 times with zero `from-cache` and zero `up-to-date` outcomes, ordered by total time spent executing (= potential savings). This is a deliberately weak, honest signal: telemetry has no machine identity, so "inputs genuinely change every build" and "task is not cacheable" are indistinguishable; the page says so. Thresholds are constants, not config.
- Server-side cache hits (`hit_count` per entry, top artifacts by hits) come from the cache index and are independent of telemetry; both views are shown on `/cache`.

### D8. Configuration summary

| Flag | Env | Default |
|---|---|---|
| `--cache-dir` | `LIGHTNING_CACHE_DIR` | `lightning-cache` next to the db file |
| `--cache-max-artifact-mb` | `LIGHTNING_CACHE_MAX_ARTIFACT_MB` | 100 |
| `--cache-max-size-mb` | `LIGHTNING_CACHE_MAX_SIZE_MB` | 10240 (10 GiB) |
| `--cache-retention-days` | `LIGHTNING_CACHE_RETENTION_DAYS` | 30 |
| `--cache-token` | `LIGHTNING_CACHE_TOKEN` | unset (writes open) |

## Risks / Trade-offs

- [Whole artifact buffered in memory per request] → bounded by the 100 MiB artifact cap and CI-scale concurrency; streaming to disk would complicate the atomic-rename path for little gain at this scale.
- [Index/filesystem divergence on crash between rename and insert] → orphan file only; startup reconcile removes it. No path serves a partial artifact.
- [LRU eviction only runs on write] → an idle over-cap cache stays over cap until the next PUT; acceptable, caps are about steady-state growth and CI writes constantly.
- [Token grants all-or-nothing write access, transmitted as Basic auth] → run behind TLS (reverse proxy), same as any Gradle remote cache credential.
- [Never-cached heuristic can flag volatile-input tasks] → documented on the page as a starting point for investigation, not a verdict.

## Migration Plan

Additive: new table via the existing `CREATE TABLE IF NOT EXISTS` startup path; new routes and flags. Existing endpoints, data, and the init script are untouched.

## Open Questions

None blocking. Deferred: machine identity in telemetry (would sharpen the cacheability heuristic), per-entry task-type attribution (protocol carries none), streaming bodies.
