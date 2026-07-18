## 1. Storage and index

- [x] 1.1 `cache_entries` table created at startup; db functions: upsert, touch/hit, totals, LRU eviction candidates, TTL prune, top entries; tests
- [x] 1.2 `cache` module: config (dir, artifact/total limits, retention, token), key validation, atomic write (temp file + rename), startup reconciliation of index vs filesystem; tests

## 2. HTTP protocol

- [x] 2.1 `GET`/`HEAD /cache/{key}`: 200 with octet-stream body / 404; hit counting on GET only; vanished file degrades to miss
- [x] 2.2 `PUT /cache/{key}`: store + index, 201; overwrite allowed; 400 invalid key; 413 over artifact limit (body limit on route)
- [x] 2.3 LRU eviction after write while total over cap; TTL prune at startup and on writes
- [x] 2.4 Optional Basic auth on PUT when token configured (password = token, username ignored); 401 + `WWW-Authenticate` otherwise; tests

## 3. Analytics UI

- [x] 3.1 db queries: overall hit rate from task_executions, never-cached task paths over last 100 builds; tests
- [x] 3.2 `/cache` page: storage stats, top artifacts by hits, overall hit rate, never-cached table with honest heuristic notes; byte formatter
- [x] 3.3 Build detail: per-build cache hit rate line; nav gains `cache` on all pages

## 4. Verification and docs

- [x] 4.1 Quality gates green: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [x] 4.2 E2E protocol via curl: PUT/GET/HEAD roundtrip, 404 miss, 400 bad keys, 413 oversize, LRU eviction with tiny cap, auth 401/2xx; record below
- [x] 4.3 E2E with real Gradle: fixture with `settings.gradle` remote cache (local cache disabled), push build, `clean`, rebuild → FROM-CACHE in Gradle output, hits in server index, telemetry hit rate on `/cache`; record below
- [x] 4.4 Phase 1–2 regression: workspace tests green, smoke curl of `/`, `/builds`, `/trends`
- [x] 4.5 README: remote cache `settings.gradle` snippet (push from CI, pull everywhere) and auth env var

## E2E record

Executed 2026-07-18 with **real Gradle 9.6.1** (phase-2 distribution reused) on JDK 21 (Corretto), debug binaries. Fixture: phase-2 two-project Gradle build in the scratchpad with `settings.gradle` gaining `buildCache { local { enabled = false }; remote(HttpBuildCache) { url; push = true; allowInsecureProtocol; credentials } }` pointing at a locally running lightning-server. Empirical note recorded in design D2: Gradle 9.6.1 pushed a **32-char lowercase hex** key (`0a2e8032…`), confirming the 32–64 hex validation range over the brief's assumed 64-char SHA-256.

Protocol via curl (server on `127.0.0.1:4243`, defaults):

- `PUT /cache/{32-hex}` → `201`; `GET` → `200`, `application/octet-stream`, byte-identical body; `HEAD` → `200`; unknown key → `404`; 64-hex key accepted → `201`.
- Bad keys (short, uppercase, non-hex, `../` traversal) → `400`, nothing stored.
- Re-PUT of an existing key → `201`, `GET` returns the new body.

Real Gradle roundtrip (with telemetry init script attached, `LIGHTNING_URL` set):

- Build 1 with `--build-cache`: `2 actionable tasks: 2 executed`, `:lib:compileJava` pushed (key visible in `cache_entries` and on disk); telemetry sent (201).
- `gradle clean`, rebuild: `> Task :lib:compileJava FROM-CACHE`, `1 executed, 1 from cache`; server index `hit_count` incremented (3 hits after three clean/rebuild cycles).
- `/cache` page: `3 entries · 1.3 KiB used of 10.00 GiB · artifact limit 100.0 MiB · retention 30 days · writes open`; overall task hit rate 9% (1 from-cache of 11 tasks needing work) rising as cycles accumulated; never-cached table lists `:lib:jar`, `:lib:clean`, `:lib:assemble`, `:lib:build` (4 executions each, no from-cache/up-to-date) — correct: `jar` and lifecycle tasks are not cacheable. Build detail shows `cache hit rate 25%`.

Auth (server on `127.0.0.1:4244`, `LIGHTNING_CACHE_TOKEN=s3cret`):

- curl `PUT` without credentials → `401` + `WWW-Authenticate: Basic realm="lightning"`; wrong password → `401`; correct token with arbitrary username → `201`; `GET` without credentials → `200`.
- Real Gradle push with wrong token in credentials: Gradle logs the 401 store failure, build still succeeds, nothing stored; with correct token the artifact lands in the index.

Eviction (server on `127.0.0.1:4245`, `--cache-max-size-mb 1 --cache-max-artifact-mb 1`):

- 2 MiB `PUT` → `413`.
- Three 400 KiB artifacts: put k1, put k2, `GET k1` (making k2 least-recently-accessed), put k3 → total over cap → k2 evicted (row and file gone, `GET` → `404`), k1 and k3 remain (`200`).
- Startup reconciliation: a stray 32-hex file and a `.tmp-*` file dropped into the cache dir disappear on restart; indexed entries and files intact.

Regression: `POST /api/runs` → `201`; `/`, `/builds`, `/trends`, `/cache`, `/runs/1` all `200`; nav on every page is `flaky · builds · trends · cache`. All 28 workspace unit tests green; fmt/clippy/test re-run clean on the final tree. Servers and Gradle daemons killed after verification.
