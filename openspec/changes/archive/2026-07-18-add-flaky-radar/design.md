## Context

Phase 1 of the lightning platform (see `docs/design/lightning-platform.md`): a flaky-test radar for Gradle/Android CI with zero build integration. A CLI uploads JUnit XML results from CI; a single-binary server stores them in SQLite, computes flaky scores, and renders an HTML UI. This is the first code in the repo, so the Cargo workspace layout is established here.

## Goals / Non-Goals

**Goals:**
- `lightning upload`: JUnit XML glob -> run payload -> POST to server, idempotent.
- `lightning-server`: ingest API, flaky score computation, 3 server-rendered HTML screens.
- Fixed data model: test identity, formal flaky definition and score, retention.
- Workspace layout ready for phases 2-4 (`crates/cli`, `crates/server`).

**Non-Goals:**
- Build telemetry, remote cache, selection (phases 2-4).
- Auth, multi-tenancy, Postgres, JS frontend, SaaS.
- Parsing formats other than JUnit XML.

## Decisions

### D1. Test identity across runs

A test is identified by `(class_name, name)`:
- `class_name` = JUnit `classname` attribute (fully qualified suite/class), falling back to the enclosing `<testsuite name>` when absent.
- `name` = JUnit `name` attribute with normalization applied.

Normalization (applied to both fields): trim, collapse internal whitespace runs to a single space, and replace volatile tokens inside the name with `_` — hex object addresses (`@[0-9a-f]{6,}`) and UUIDs. Parameterized test invocations (e.g. `test[2]`, `test[input=foo]`) keep their parameter string: each parameter set is a distinct test, because different parameters genuinely exercise different behavior. Volatile-token replacement is what keeps identity stable when parameters embed `toString()` of objects.

Alternative considered: stripping `[...]` suffixes entirely (one identity per parameterized method). Rejected: it would blend a genuinely failing parameter set with passing siblings and produce false flaky signals.

### D2. Verdicts and run-level status

Per execution, status is one of `pass`, `fail`, `error`, `skip` (from JUnit children; `error` is treated as `fail` everywhere in scoring). A test may appear multiple times in one run (Gradle retries, sharded suites). The run-level verdict per test:
- all executions pass -> `pass`
- all executions fail/error -> `fail`
- both present -> `mixed` (in-run retry flip — direct flaky evidence)
- only skips -> excluded from scoring

### D3. Formal flaky definition and score

Scoring window: the last **50** run-level verdicts of a test (ordered by run creation time), skips excluded. Within the window compute:
- `flip_shas` = number of distinct commit SHAs that have both passing and failing verdicts (a `mixed` verdict counts its SHA immediately) — same-SHA flips, via retries in one run or across runs/uploads on the same SHA.
- `flips` = number of adjacent transitions between `pass` and `fail` in the verdict sequence (`mixed` verdicts are excluded from the sequence; they are already counted via `flip_shas`).
- `n` = number of verdicts in the window.

**Definition: a test is flaky iff `flip_shas >= 1` OR `flips >= 2`.** A single flip across SHAs is not flaky — it is consistent with an honest break or fix; two or more flips mean the verdict returned without a same-SHA proof.

**Score** (0-100, 0 = not flaky):

```
score = round(100 * min(1.0, 0.6 * min(flip_shas, 3) / 3 + 0.4 * flips / max(n - 1, 1)))
```

Same-SHA flips dominate (hard evidence, saturating at 3 SHAs); cross-SHA flip rate contributes the rest. Computed on read — no materialized scores at this scale.

Alternative considered: probabilistic models (beta-binomial on retry outcomes). Rejected: opaque, and the platform principle is deterministic and explainable.

### D4. Idempotent uploads

The client computes a `run_key`:
1. explicit `--run-key` flag, else
2. GitHub Actions: `gh:{GITHUB_REPOSITORY}:{GITHUB_RUN_ID}:{GITHUB_RUN_ATTEMPT}`, else
3. `local:` + blake3 hash of SHA, branch and the canonicalized parsed results.

The server has `UNIQUE(run_key)`; re-posting an existing key returns the existing run id with `deduplicated: true` and writes nothing. Two CI attempts on the same SHA get distinct keys and are distinct runs (that is what produces same-SHA evidence).

### D5. Retention

Configurable `--retention-days` (env `LIGHTNING_RETENTION_DAYS`), default **90**. On startup and after every successful ingest the server deletes runs older than the cutoff and their results, then removes orphaned test rows. Rationale: the scoring window is 50 runs; 90 days of raw history is ample for trends and keeps SQLite small without a background scheduler.

### D6. Tech stack

- CLI: `clap`, `quick-xml` (manual pull parsing of `testsuite`/`testcase`), `glob` (supports `**`), `ureq` (sync HTTP; no tokio in the CLI), `serde`/`serde_json`, `blake3`.
- Git info: env first (`GITHUB_SHA`, `GITHUB_REF_NAME`), fallback to `git rev-parse` via `std::process::Command`. `gix` is deferred until phase 4 needs it.
- Server: `axum` + `tokio`, `rusqlite` (bundled SQLite) behind a `Mutex<Connection>` — single-writer SQLite makes a connection pool pointless at this scale. HTML rendered with plain Rust formatting plus an escape helper; no template engine, no JS build chain.
- Shared types: duplicated small payload structs in each crate rather than a shared crate — one JSON contract, two structs; a `crates/types` crate is not yet justified.

### D7. Schema

```sql
runs(id INTEGER PRIMARY KEY, run_key TEXT UNIQUE NOT NULL, sha TEXT NOT NULL,
     branch TEXT NOT NULL, ci_url TEXT, created_at TEXT NOT NULL)
tests(id INTEGER PRIMARY KEY, class_name TEXT NOT NULL, name TEXT NOT NULL,
      UNIQUE(class_name, name))
results(id INTEGER PRIMARY KEY, run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
        test_id INTEGER NOT NULL REFERENCES tests(id), status TEXT NOT NULL,
        time_ms INTEGER NOT NULL, message TEXT)
```

## Risks / Trade-offs

- [Score computed on read gets slow with many tests] -> window capped at 50 verdicts, indexed lookups; materialize later if real deployments need it.
- [Volatile-token normalization misses some patterns] -> unmatched volatility only splits identities (extra rows), never merges distinct tests; regexes can be extended without migration.
- [Single `Mutex<Connection>` serializes requests] -> acceptable for a self-hosted single-team radar; ingest is one transaction per run.
- [`local:` run_key hashes payload — flaky reruns of the same local invocation create distinct runs when results differ] -> intended: differing results are new evidence, identical results dedupe.

## Migration Plan

Greenfield; no migration. Schema is created on server startup (`CREATE TABLE IF NOT EXISTS`).

## Open Questions

None blocking. Deliberate deferrals: PR-number extraction beyond GitHub Actions, score materialization, shared types crate.
