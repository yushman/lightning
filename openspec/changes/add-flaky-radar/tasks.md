## 1. Workspace scaffolding

- [x] 1.1 Create Cargo workspace at repo root with `crates/cli` (binary `lightning`) and `crates/server` (binary `lightning-server`)
- [x] 1.2 Verify `cargo build --workspace` succeeds

## 2. CLI: lightning upload

- [x] 2.1 JUnit XML parser (quick-xml): testsuite/testcase, statuses pass/fail/error/skip, duration, failure message; unit tests on fixture XML
- [x] 2.2 Report discovery by glob with default `**/build/test-results/**/*.xml`, `--glob` override
- [x] 2.3 Git/CI metadata: GITHUB_SHA/GITHUB_REF_NAME, git fallback via `git rev-parse`, `--sha`/`--branch` overrides, CI URL from GitHub Actions env
- [x] 2.4 run_key computation: `--run-key` > GitHub Actions identity > blake3 hash of sha+branch+results; unit test
- [x] 2.5 POST payload to `<server>/api/runs` (ureq), clear errors on no reports / unreachable server

## 3. Server: ingest and storage

- [x] 3.1 SQLite schema (runs, tests, results) created at startup; rusqlite behind Mutex
- [x] 3.2 Test identity normalization (whitespace collapse, hex-address and UUID tokens -> `_`); unit tests
- [x] 3.3 `POST /api/runs`: transactional insert, 400 on malformed payload, dedup by run_key with indicator; tests
- [x] 3.4 Retention pruning (default 90 days, flag/env) at startup and after ingest; test

## 4. Flaky scoring

- [x] 4.1 Run-level verdict derivation (pass/fail/mixed, skip excluded); unit tests
- [x] 4.2 Score over 50-verdict window: flip_shas, flips, flaky definition, formula; unit tests covering same-SHA flip, honest regression (score 0), cross-SHA alternation
- [x] 4.3 `GET /api/flaky` JSON ordered by score desc

## 5. Web UI

- [x] 5.1 HTML escape helper and shared page shell (inline CSS)
- [x] 5.2 `/` flaky list with scores and verdict trend, links to test pages
- [x] 5.3 `/tests/{id}` verdict history with SHA/branch/time and run links
- [x] 5.4 `/runs/{id}` run summary: metadata, counts, failed/mixed tests

## 6. Verification and docs

- [x] 6.1 Quality gates green: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [x] 6.2 E2E: start server, fixture JUnit XMLs with flaky pattern (same-SHA flip across two uploads + cross-SHA flips), `lightning upload` twice for idempotency, curl API and all three HTML screens asserting content; record commands and results below
- [x] 6.3 README.md: what lightning is, run server, add upload to CI

## E2E record

Executed 2026-07-18 against debug binaries (`cargo build --workspace`), fixtures in a scratch dir.

Fixtures: four run dirs, each `app/build/test-results/test/TEST-com.example.xml` with three tests:
- `com.example.FlakyTest#sometimesFails` — fail on sha aaa1111 (run-a1), pass on aaa1111 (run-a2), fail on bbb2222 (run-b), pass on ccc3333 (run-c): same-SHA flip across two uploads plus cross-SHA flips.
- `com.example.StableTest#alwaysPasses` — pass everywhere.
- `com.example.RegressTest#breaks` — pass, pass, fail, fail (honest regression, must not be flaky).

Commands:

```
lightning-server --addr 127.0.0.1:4141 --db $S/e2e/lightning.db &
export LIGHTNING_SERVER=http://127.0.0.1:4141
(cd run-a1 && lightning upload --run-key run-a1 --sha aaa1111 --branch main)   # uploaded 3 results as run 1
(cd run-a2 && lightning upload --run-key run-a2 --sha aaa1111 --branch main)   # uploaded 3 results as run 2
(cd run-b  && lightning upload --run-key run-b  --sha bbb2222 --branch main)   # uploaded 3 results as run 3
(cd run-c  && lightning upload --run-key run-c  --sha ccc3333 --branch main)   # uploaded 3 results as run 4
(cd run-a1 && lightning upload --run-key run-a1 --sha aaa1111 --branch main)   # run already uploaded (run 1, key run-a1)
```

Verified via HTTP:
- `GET /api/flaky` -> exactly one item: `com.example.FlakyTest#sometimesFails`, `score: 60`, `flip_shas: 1`, `flips: 3` (formula: 0.6*1/3 + 0.4*3/3 = 0.6).
- SQLite after 5 uploads: 4 runs, 12 results (idempotent re-upload wrote nothing).
- `GET /` contains the flaky test, `<td class="score">60</td>`, verdict trend blocks; `RegressTest` absent (honest regression not flagged).
- `GET /tests/1` mentions `aaa1111` twice (fail and pass on the same SHA visible in history).
- `GET /runs/1` shows `2 passed · 1 failed` and links the failing test.
- `GET /tests/999` -> 404; malformed `POST /api/runs` (`{"sha":"x"}`) -> 400.

Server killed after verification. Quality gates re-run on the final tree: fmt/clippy/test all green.
