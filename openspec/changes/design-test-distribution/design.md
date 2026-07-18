# Test distribution — design

## Context

Phase 5 of the lightning platform (`docs/design/lightning-platform.md` §4, "северная звезда"): distributed execution of Gradle tests across a fleet of workers. This document is the groomed design; implementation is deferred until phases 1–4 mature (see Prerequisites). It builds strictly on what is shipped:

- **Phase 1** — `lightning upload` → server ingest → per-test history (`tests`, `results` with `time_ms`), deterministic flaky scores, idempotent `run_key` dedup.
- **Phase 2** — telemetry init script → per-task wall-clock durations and outcomes per build.
- **Phase 3** — Gradle HTTP build cache in the same server, LRU/TTL bounded, optional shared-token write auth.
- **Phase 4** — `lightning.lock`: modules, source dirs, per-module task lists (knows `:app` runs `testDebugUnitTest` while `:lib` runs `test`), typed main/test edges; `affected` closure with FN-never degradation; `--format github-matrix` fan-out.

Platform principles that bind this design: integration stays outside the build (init scripts are the sanctioned maximum depth), false negatives never, single Rust binary + SQLite, each phase ships standalone value.

## Problem and prior art

A selected, cached CI run is still bounded by its slowest test task. On a 20+ module Android monorepo, `:app:testDebugUnitTest` alone can dominate the critical path; Gradle parallelizes across modules on one machine but a single JVM/machine saturates. The fix is horizontal: run test work on N machines and merge results.

**Develocity Test Distribution** (the only comparable product) works *inside* the build: a Gradle plugin replaces the `Test` task's executer, discovers test classes from the compiled classpath, streams individual classes to remote agents over a persistent connection, ships the tests' classpath as content-addressed artifacts to agents, and merges results back into the same task so reports/tooling see one local `Test` task. That yields class-level granularity and zero CI-workflow changes, at the price of deep build ownership: it must track AGP/Gradle internals across versions — exactly the moat (`ров AGP`) lightning refuses to enter.

**The "outside the build" tension, stated honestly.** Test distribution cannot be 100% outside the build: tests execute inside Gradle on some machine, and *class-level* narrowing needs the build to apply a filter. What *can* stay outside is everything else: deciding what to run, where, in which order; moving results; balancing; retrying; accounting. The parts of Develocity's design that force build ownership are (a) replacing the test executer and (b) shipping the classpath instead of building on the worker. Both are avoidable:

- (a) is avoided by making the **work unit a whole Gradle invocation**, not a test class handed to a foreign executer. Workers run `./gradlew :m:testDebugUnitTest [filters]` — stock Gradle, stock AGP, zero execution ownership.
- (b) is avoided by giving workers the **source at the session SHA plus the phase-3 remote cache**: the worker's compile tasks hit the cache, so "shipping the classpath" degenerates to cache pulls we already serve.

The one genuinely-inside residue is class-level *exclusion* (stage 3): Gradle's CLI `--tests` flag is include-only, so a "remainder" shard that must run *everything not assigned elsewhere* cannot be expressed from the command line. Resolution: the exclusion filter is applied by an **init script** (`tasks.withType(Test) { filter.excludeTestsMatching(...) }` driven by an env var the worker CLI sets) — the same integration depth as phase-2 telemetry, configuration-level only, no executer replacement, no AGP internals. This is the explicit, bounded exception, and stages 1–2 do not need even that.

## Goals / Non-Goals

**Goals (of the eventual implementation; this change ships only the design):**

- Cut test wall-clock time roughly linearly with worker count for the module-parallel portion of the suite.
- Reuse the platform: phase-4 lock for units, phase-1/2 history for balancing, phase-3 cache for worker warm-up, phase-1 ingest/flaky radar for results — one server binary, SQLite, no broker.
- FN-never analog for execution: a scheduled test is never silently dropped; the session verdict is trustworthy or explicitly `incomplete`.
- Each delivery stage independently shippable and useful.

**Non-Goals (v1 of the eventual implementation):**

- Replacing or wrapping Gradle's test executer; any per-test-method distribution.
- Android instrumented/device tests (`connectedAndroidTest`, device farms) — JVM unit tests only.
- Managed worker provisioning, autoscaling, or a SaaS fleet; workers are user-provided machines or CI jobs.
- Shipping compiled classpaths to workers (the remote cache is the artifact channel).
- Cross-session "skip tests that passed on this code before" (predictive test selection) — a separate future capability.
- Non-test task distribution (remote build execution generally).

## Decisions

### D1. Work unit and integration boundary

A **work unit** is one Gradle invocation on one worker: `(module path, task name, optional class filter)`. Granularity per stage:

- Stages 1–2: unit = `:module:testTask` (task names resolved from the phase-4 lock exactly as `lightning run` does).
- Stage 3: unit = module test task **narrowed to a class set** — include shards get `--tests 'com.x.FooTest' --tests ...` on the command line (still zero build integration); the one remainder shard per split module gets an *exclude* list applied via the init-script channel (see tension resolution above).

Rejected alternative: a custom JUnit-platform launcher on workers running classes directly against a classpath we assemble. It would drop Gradle from the worker entirely — but assembling the correct test classpath/runtime (AGP variants, resources, JVM args, Robolectric) *is* the moat; honest refusal.

### D2. Components and CLI surface

Three pieces, all in the two existing binaries:

- **Scheduler** — new module in `lightning-server`: distribution sessions, unit ledger, lease queue in SQLite, dispatch/heartbeat/complete endpoints, session status API, a `/dist` UI page. No new process, no broker; the queue is a table (`UPDATE ... WHERE lease_expired ... RETURNING`-style claim under the existing connection lock).
- **Worker agent** — `lightning worker` subcommand: long-polls the server for a unit, checks out the session SHA (fetch into a persistent working copy, or fresh clone on ephemeral runners), runs Gradle with the unit's task+filters plus the telemetry init script and remote cache enabled, then posts the unit's JUnit XML and status back. `--drain` exits when the queue for its session/labels is empty (ephemeral CI-matrix mode); default mode loops forever (persistent pool mode).
- **Orchestrator** — `lightning test-dist [<task>] [selection flags]` subcommand: computes affected modules from the lock (same selection semantics and flags as `affected`/`run`), asks the server to create a session (units + weights + session SHA + labels), polls session status streaming progress, exits with the merged verdict. Exit codes follow the `affected` family conventions: 0 all green, 1 error, 3 nothing affected, 4 stale lock, plus 5 = test failures, 6 = session incomplete.

Decision: a **new subcommand**, not an extension of `run`. `run` is local-execution sugar with "exit code = Gradle's" semantics and no server dependency; overloading it with server sessions, tokens and polling would break its contract. `test-dist` shares the selection flags and lock plumbing instead.

### D3. Discovery and partitioning

**Module-level (stages 1–2).** Units come straight from the lock: affected modules × resolved test task. No discovery problem exists — the lock already has tasks per module.

**Class-level (stage 3).** Test classes are discovered from **phase-1 history**, not from the build: every class that ever reported a result for module M (see prerequisite P1, module attribution) is a known class with a known cumulative duration. The partitioner splits a module only when its expected duration exceeds a threshold (e.g. > 2× target shard time), producing K include-shards (greedy LPT over per-class durations) plus **one remainder shard** carrying the exclude list of all classes assigned to the K include-shards.

Totality argument (FN-never): every known class is in exactly one include shard; every *unknown* class — new, renamed, or simply never seen by history — is not in any exclude list, therefore runs in the remainder shard **by construction**. History staleness can only misbalance, never drop. Deleted classes make an include filter match nothing; Gradle's "no tests found for filter" failure is downgraded by the worker to an empty-but-successful unit (recognized case, logged), so deletions do not fail sessions.

**Ordering.** Units are dispatched longest-expected-first (LPT): with a dynamic queue this alone yields near-optimal makespan without any static assignment.

### D4. Dispatch protocol

Plain HTTP + JSON on the existing axum server; long-poll instead of WebSockets (fits `ureq` in the CLI, proxies, and the single-binary principle):

- `POST /api/dist/sessions` (orchestrator) → `{session_id}`. Body: sha, branch, run identity (for the merged run's `run_key`), repo hint, required labels, units `[{module, task, include_filters?, exclude_filters?, weight_ms}]`.
- `POST /api/dist/lease` (worker) → long-poll up to ~30 s; body: worker id, labels, optional session pin. Response: a unit + `lease_id` + lease TTL, or `204` (drain workers exit on `204` + session terminal).
- `POST /api/dist/units/{id}/heartbeat` (worker, every TTL/3) → extends the lease; `410` tells a worker its lease was revoked (stop and discard).
- `POST /api/dist/units/{id}/complete` (worker) → status `completed|infra_failed`, Gradle exit code, log tail, and the unit's parsed test results (same JSON shape as the phase-1 upload payload).
- `GET /api/dist/sessions/{id}` (orchestrator poll) → per-unit states, progress, final verdict.

Delivery semantics are **at-least-once**: a lease can expire while a slow worker still finishes, so two attempts of one unit may both complete. The ledger keys results by `(unit, attempt)`; the first `complete` accepted for a unit wins, later ones are acknowledged and discarded. Duplicate execution is wasted compute, never wrong results — consistent with the platform's FP-tolerant direction.

Auth reuses the phase-3 shared-token pattern: `--dist-token` / `LIGHTNING_DIST_TOKEN` on the server; when set, all `/api/dist/*` calls require it (Bearer). Without a token the endpoints are open, matching cache-write semantics for trusted networks. Workers execute repository code by design — they must be trusted exactly like CI runners; the token gates *who may enqueue and lease work*, it is not a sandbox (documented loudly).

### D5. Result merge and flaky-radar synergy

Workers do **not** call `lightning upload`. Phase-1 ingest is idempotent by `UNIQUE(run_key)` — "re-posting an existing key returns the existing run and writes nothing" — so N shards uploading under one key would drop N−1 of them, and N distinct keys would shatter one CI run into N runs in the radar. Instead, shard results travel inside `complete`, and **the server materializes exactly one run** through the existing internal ingest path when the session reaches a terminal state: `run_key = dist:{session_key}`, results = union of all accepted attempts' results. Consequences, all free:

- The flaky radar sees one run per CI run, same as today; retries *within* a worker's Gradle invocation still produce `mixed` verdicts (same-SHA evidence) exactly as phase 1 defines.
- Session re-creation for the same CI attempt dedups by `run_key` as today.
- Results from `infra_failed` attempts are **discarded**, not merged: a crashed JVM's partial XML must not fabricate pass/fail evidence. Only `completed` attempts feed the run.
- Per-unit Gradle telemetry still flows through the ordinary phase-2 init script from each worker (separate build documents — correct, since they *are* separate builds).

### D6. Balancing and cold start

Expected duration of a unit, best source first:

1. **Class-filtered unit**: sum of per-class recent median durations from phase-1 `results.time_ms` (per-class = sum of its tests' medians over the last N runs on the default branch).
2. **Whole-module unit**: recent median of the module's test-task duration from phase-2 `task_executions` — preferred over summed test times because it includes JVM startup, Robolectric warm-up, and task overhead that per-test times miss.
3. **Cold start** (no history for the task/module): weight = a fixed default (e.g. 60 s). No class splitting is ever attempted without class history; cold modules are simply whole-module units. First real runs populate both histories; balance converges after one or two sessions.

Static planning (stage 1) packs units into a requested number of bins with greedy LPT. Dynamic mode (stage 2+) needs no packing — LPT dispatch order plus work-stealing-by-polling balances naturally; weights only order the queue and size lease TTLs/timeouts.

### D7. Worker model

**Provisioning — two modes, one protocol** (the server cannot tell them apart):

- *Ephemeral, CI-spawned*: a GitHub Actions matrix job per worker slot running `lightning worker --session <id> --drain`. Zero standing infrastructure — the natural on-ramp, and consistent with the platform assumption of ephemeral runners.
- *Persistent pool*: user-managed machines running `lightning worker` as a service. Keeps warm Gradle daemons, working copies, and local caches → dramatically lower per-unit overhead; this is where distribution beats plain CI fan-out.

**Environment reproducibility** is explicitly the operator's contract, not lightning's: workers must provide the toolchain (JDK, Android SDK, accepted licenses) the build needs — the same contract every CI runner already fulfills. Lightning's part is *matching, not provisioning*: workers self-declare labels (`--label jdk17 --label android-sdk`), sessions declare required labels, the scheduler leases only on label superset. v1 labels are free-form user strings; no automatic environment fingerprinting (recorded as an open question).

**Source transfer**: workers fetch the repo themselves at the session SHA (deploy key / CI-provided token). Rejected alternative — server-mediated source or classpath bundles — would turn the server into an artifact CDN and re-enter classpath-assembly territory. Consequence, stated honestly: workers need read access to the repository, and a PR's SHA must be fetchable (`refs/pull/*/head` on GitHub); both documented as setup requirements.

**Artifact warm-up**: workers run with the phase-3 remote cache (`pull` mode) — compile/prepare tasks upstream of the test task become cache hits when CI has already pushed them for that SHA. No new transfer channel exists in v1; if trunk builds keep the cache warm, a worker's marginal work approaches "run the tests".

### D8. Failure semantics and the execution ledger

**Invariant (FN-never for execution): every unit the session created reaches a terminal state, and the session verdict enumerates them; a test is never silently dropped.** Mechanism: the SQLite ledger row per unit — `pending → leased → completed | infra_failed(attempt) → …` — with the session terminal state derived, never asserted:

- `passed` — all units completed, zero test failures.
- `failed` — all units completed, ≥ 1 test failure (named).
- `incomplete` — ≥ 1 unit exhausted `max_attempts` (default 2) or the session wall-clock timeout fired; the verdict names every unaccounted unit and the orchestrator exits 6, distinctly from test failure. CI must treat it as red.

Rules:

- **Infra failure vs test failure**: a unit whose Gradle run finishes and yields parseable XML is `completed` — even with failing tests. Test failures are *results*, and the scheduler never re-runs them: retry-for-flakiness stays where phase 1 put it (in-build retries produce `mixed` verdicts; the radar reports, never hides). Only infra failures — lease expiry, worker crash, non-zero exit with no XML, checkout failure — requeue the unit, preferring a different worker on the next attempt.
- **Worker dies mid-unit**: heartbeats stop → lease expires (TTL ≈ 3× expected duration, clamped to [5 min, unit timeout]) → attempt recorded `infra_failed`, unit requeued. If the "dead" worker later completes, at-least-once dedup applies (D4).
- **Timeouts**: per-unit hard timeout = max(3× expected, 15 min), enforced worker-side (kill Gradle, report `infra_failed`/`timeout`) and server-side via lease expiry as the backstop; plus a session wall-clock timeout (default 60 min) so an empty worker pool cannot hang CI forever — it degrades to `incomplete` with "no workers leased" diagnostics.
- **Coverage soft check**: at merge time the server diffs reported classes per module against recent history; classes in history but absent from the session surface as a UI warning (deletions make this advisory, not an invariant — the *hard* invariant is the unit ledger, whose totality argument is D3's construction).
- **Orchestrator dies**: the session is server-owned and runs to terminal state regardless; a re-run of the CI step re-attaches by session key instead of double-creating (same idempotency key as the merged run).

### D9. Staged delivery

Three shippable slices, each its own future OpenSpec change, each valuable without the next:

- **Stage 1 — static timing-balanced sharding (near-pure reuse).** `lightning affected --format github-matrix --shards <n> [--task test]`: instead of one matrix entry per module, emit `n` (or fewer) balanced bins — `{"include":[{"shard":"1","tasks":":app:testDebugUnitTest :core:test"}, …]}` — packed by phase-2 task-timing history (D6), task names from the lock. CI runs `./gradlew ${{ matrix.tasks }}` per shard. No scheduler, no workers, no new server surface; needs only read access to timing data (CLI gains one server GET, degrading to weight-1 packing offline). Value: capped, balanced fan-out where phase 4 today gives one job per module.
- **Stage 2 — dynamic distribution (module-level).** Scheduler in the server (sessions, ledger, lease protocol — D4, D8), `lightning worker --drain` for CI-matrix workers and the persistent-pool mode, `lightning test-dist` orchestrator, server-side merge to one flaky-radar run (D5), `/dist` session page in the UI. Work stealing makes stragglers self-correct — the win over stage 1's static bins.
- **Stage 3 — class-level sharding.** Split oversized modules by class history with the remainder-shard construction (D3), include filters via `--tests`, exclude filters via the init-script channel (D1), label-based routing hardening, persistent-pool ergonomics. Value: the largest module stops being the makespan floor.

Not scheduled (v2+): instrumented tests, predictive selection, method-level splitting.

### D10. OpenSpec artifacts for a design-only change

The project schema (`spec-driven`) defines four artifacts: proposal → specs → design → tasks. Phase-5 DoD requires strict validation but forbids implementation and archiving. Resolution: ship all four artifacts — proposal and design as the real deliverables; `specs/test-distribution/spec.md` as the **target contract** for the future stages (explicitly labeled deferred; it reaches `openspec/specs/` only via those stages' own changes, never via this one, since this change is never archived); `tasks.md` as the staged implementation outline with every task unchecked and marked deferred. This keeps `openspec validate design-test-distribution --strict` green without pretending any work item is actionable today.

## Prerequisites (phases 1–4 maturation)

- **P1 — module attribution for test results (phase 1).** `tests`/`results` carry no module; class-level balancing and the coverage soft check need "class → module". Additive change: `upload` derives the module from the report path (`<module-dir>/build/test-results/...` matched against the lock or path heuristics) and ingest stores it. Without P1, stage 3 is blocked; stages 1–2 are not.
- **P2 — timing-history read API (phase 2).** Stage 1 needs per-task recent medians over the wire (e.g. `GET /api/timings?tasks=...`); today timings are UI-only. Small additive endpoint.
- **P3 — trunk keeps the cache warm (phase 3, operational).** Worker economics assume compile tasks hit the remote cache; the `/cache` analytics should first demonstrate a healthy hit rate on default-branch builds.
- **P4 — lock trust in anger (phase 4, operational).** Task-name resolution and affected closure must have survived real monorepo use (Android task-name divergence, `srcDir` edge cases) before test-dist builds on them.
- **P5 — auth posture review.** Three token knobs (cache, dist, future ingest) suggest consolidating into one server token story before adding the third; decide at stage-2 grooming.

## Risks / Trade-offs

- [Worker checkout + configuration overhead dwarfs test time on small modules] → persistent pools amortize it (warm daemon, incremental fetch); scheduler only splits modules above a duration threshold; stage 1 has zero such overhead — adoption can stop there profitably.
- [SQLite single-writer as a queue] → lease claims are single-row transactions under the existing connection lock; a worker fleet of tens polling at long-poll cadence is trivial write load. Revisit only with evidence.
- [At-least-once double execution skews flaky evidence] → duplicates are discarded at merge by first-accepted-attempt (D4); only in-invocation retries create `mixed` verdicts, unchanged from phase 1.
- [History-driven class filters go stale] → staleness only misbalances; the remainder-shard construction (D3) makes dropping impossible, and "no tests matched" on deleted classes is a recognized, non-fatal case.
- [Init-script exclude channel meets Gradle configuration cache] → filter values enter via env var read at configuration time, which invalidates the configuration cache per shard; acceptable on workers (documented), and only stage 3 pays it.
- [Label matching is honor-system] → wrong-environment failures surface as infra failures attributed to a worker id; v1 accepts this, environment fingerprinting is an open question.
- [Session merge makes the server a required dependency of the test verdict] → it already is for flaky/telemetry data, but test-dist makes it *blocking*; the session wall-clock timeout plus `incomplete` verdict keep failure modes explicit rather than hanging.

## Open Questions

- Environment fingerprinting: should workers auto-report JDK/SDK versions (reusing telemetry capture) and the scheduler match on them, instead of free-form labels?
- Private-repo source access ergonomics: document deploy-key patterns only, or add a server-mediated "fetch bundle" as an opt-in for locked-down networks (re-opens the artifact-channel decision)?
- Should stage 2 sessions support heterogeneous tasks (lint, detekt) since the unit model is task-generic, or stay test-only until the ledger semantics are proven?
- `run_key` collision policy when a CI re-run (same attempt) re-creates a session after a partial merge — reuse-and-extend vs strict dedup (leaning strict, matching phase 1).
- Retention for session/ledger rows (likely: same 90-day run retention, cascading).
