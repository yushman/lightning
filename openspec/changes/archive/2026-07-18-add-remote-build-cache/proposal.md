## Why

Phases 1–2 observe CI (flaky tests, build telemetry) but do not speed it up. Phase 3 of the lightning platform adds the first acceleration feature: a remote Gradle build cache served by the same single binary, plus cache analytics on top of phase-2 telemetry — the differentiator against Gradle's free Build Cache Node, which is a bare cache with no insight into hit rates or cacheability.

## What Changes

- Server implements the open Gradle HTTP build cache protocol: `GET`/`HEAD`/`PUT /cache/{key}` with opaque binary bodies (`application/octet-stream`). Artifacts are stored as files in a configurable cache directory (default next to the SQLite db) and indexed in SQLite (key, size, timestamps, hit count).
- Limits and retention: configurable max artifact size (default 100 MiB, oversized PUTs get 413), max total cache size (default 10 GiB) enforced by LRU eviction on write, and a last-access TTL (default 30 days) pruned at startup and on writes.
- Optional write protection: a shared token (`LIGHTNING_CACHE_TOKEN`); when set, `PUT` requires HTTP Basic auth with the token as password (Gradle's native credentials mechanism). Reads stay open.
- Cache analytics in the web UI: a `/cache` page with storage stats (size used, entry count, top artifacts by hits), overall task cache hit rate from telemetry, and a "never cached task paths" table (simple, documented heuristic over recent builds). Build detail gains a per-build hit rate. Navigation updated.
- README documents the `settings.gradle` snippet (push from CI, pull everywhere) and the auth env var.

## Capabilities

### New Capabilities

- `remote-build-cache`: the HTTP cache protocol endpoints, key validation, storage layout, size limits, LRU eviction, TTL retention, and optional write auth.

### Modified Capabilities

- `web-ui`: new cache analytics screen, per-build cache hit rate on build detail, navigation extended.

## Impact

- Server: new `cache` module, `cache_entries` table, `/cache/{key}` routes, `/cache` page, new config flags/env vars (`LIGHTNING_CACHE_DIR`, `LIGHTNING_CACHE_MAX_ARTIFACT_MB`, `LIGHTNING_CACHE_MAX_SIZE_MB`, `LIGHTNING_CACHE_RETENTION_DAYS`, `LIGHTNING_CACHE_TOKEN`). One new dependency (`base64`, for Basic auth parsing).
- CLI and init script: unchanged — the remote cache is enabled in `settings.gradle`, not via init script.
- No changes to phase 1–2 endpoints or data.
