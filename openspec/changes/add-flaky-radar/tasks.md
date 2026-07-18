## 1. Workspace scaffolding

- [ ] 1.1 Create Cargo workspace at repo root with `crates/cli` (binary `lightning`) and `crates/server` (binary `lightning-server`)
- [ ] 1.2 Verify `cargo build --workspace` succeeds

## 2. CLI: lightning upload

- [ ] 2.1 JUnit XML parser (quick-xml): testsuite/testcase, statuses pass/fail/error/skip, duration, failure message; unit tests on fixture XML
- [ ] 2.2 Report discovery by glob with default `**/build/test-results/**/*.xml`, `--glob` override
- [ ] 2.3 Git/CI metadata: GITHUB_SHA/GITHUB_REF_NAME, git fallback via `git rev-parse`, `--sha`/`--branch` overrides, CI URL from GitHub Actions env
- [ ] 2.4 run_key computation: `--run-key` > GitHub Actions identity > blake3 hash of sha+branch+results; unit test
- [ ] 2.5 POST payload to `<server>/api/runs` (ureq), clear errors on no reports / unreachable server

## 3. Server: ingest and storage

- [ ] 3.1 SQLite schema (runs, tests, results) created at startup; rusqlite behind Mutex
- [ ] 3.2 Test identity normalization (whitespace collapse, hex-address and UUID tokens -> `_`); unit tests
- [ ] 3.3 `POST /api/runs`: transactional insert, 400 on malformed payload, dedup by run_key with indicator; tests
- [ ] 3.4 Retention pruning (default 90 days, flag/env) at startup and after ingest; test

## 4. Flaky scoring

- [ ] 4.1 Run-level verdict derivation (pass/fail/mixed, skip excluded); unit tests
- [ ] 4.2 Score over 50-verdict window: flip_shas, flips, flaky definition, formula; unit tests covering same-SHA flip, honest regression (score 0), cross-SHA alternation
- [ ] 4.3 `GET /api/flaky` JSON ordered by score desc

## 5. Web UI

- [ ] 5.1 HTML escape helper and shared page shell (inline CSS)
- [ ] 5.2 `/` flaky list with scores and verdict trend, links to test pages
- [ ] 5.3 `/tests/{id}` verdict history with SHA/branch/time and run links
- [ ] 5.4 `/runs/{id}` run summary: metadata, counts, failed/mixed tests

## 6. Verification and docs

- [ ] 6.1 Quality gates green: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] 6.2 E2E: start server, fixture JUnit XMLs with flaky pattern (same-SHA flip across two uploads + cross-SHA flips), `lightning upload` twice for idempotency, curl API and all three HTML screens asserting content; record commands and results below
- [ ] 6.3 README.md: what lightning is, run server, add upload to CI

## E2E record

(to be filled during 6.2)
