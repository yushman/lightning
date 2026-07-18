## 1. Storage and index

- [ ] 1.1 `cache_entries` table created at startup; db functions: upsert, touch/hit, totals, LRU eviction candidates, TTL prune, top entries; tests
- [ ] 1.2 `cache` module: config (dir, artifact/total limits, retention, token), key validation, atomic write (temp file + rename), startup reconciliation of index vs filesystem; tests

## 2. HTTP protocol

- [ ] 2.1 `GET`/`HEAD /cache/{key}`: 200 with octet-stream body / 404; hit counting on GET only; vanished file degrades to miss
- [ ] 2.2 `PUT /cache/{key}`: store + index, 201; overwrite allowed; 400 invalid key; 413 over artifact limit (body limit on route)
- [ ] 2.3 LRU eviction after write while total over cap; TTL prune at startup and on writes
- [ ] 2.4 Optional Basic auth on PUT when token configured (password = token, username ignored); 401 + `WWW-Authenticate` otherwise; tests

## 3. Analytics UI

- [ ] 3.1 db queries: overall hit rate from task_executions, never-cached task paths over last 100 builds; tests
- [ ] 3.2 `/cache` page: storage stats, top artifacts by hits, overall hit rate, never-cached table with honest heuristic notes; byte formatter
- [ ] 3.3 Build detail: per-build cache hit rate line; nav gains `cache` on all pages

## 4. Verification and docs

- [ ] 4.1 Quality gates green: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] 4.2 E2E protocol via curl: PUT/GET/HEAD roundtrip, 404 miss, 400 bad keys, 413 oversize, LRU eviction with tiny cap, auth 401/2xx; record below
- [ ] 4.3 E2E with real Gradle: fixture with `settings.gradle` remote cache (local cache disabled), push build, `clean`, rebuild → FROM-CACHE in Gradle output, hits in server index, telemetry hit rate on `/cache`; record below
- [ ] 4.4 Phase 1–2 regression: workspace tests green, smoke curl of `/`, `/builds`, `/trends`
- [ ] 4.5 README: remote cache `settings.gradle` snippet (push from CI, pull everywhere) and auth env var
