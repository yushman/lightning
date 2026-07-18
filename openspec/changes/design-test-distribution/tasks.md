# Tasks — deferred implementation outline

> **All tasks below are deferred.** Phase 5 is design-only: this change is never applied or archived. Each stage becomes its own future OpenSpec change once the prerequisites in `design.md` are met; the lists here are the groomed starting point for those changes, not work to execute now.

## 1. Stage 1 — static timing-balanced sharding (future change)

- [ ] 1.1 P2 prerequisite: additive `GET /api/timings` endpoint serving recent per-task medians
- [ ] 1.2 `affected --shards <n> [--task <t>]`: LPT bin packing by history, lock-resolved task names, github-matrix `{shard, tasks}` entries; offline/cold fallback to uniform weights
- [ ] 1.3 Property test: shards partition affected modules exactly (no drop, no dup) for random graphs and weights
- [ ] 1.4 README: sharded fan-out workflow example

## 2. Stage 2 — dynamic distribution, module-level (future change)

- [ ] 2.1 P1 prerequisite: module attribution on upload/ingest (additive column, path-derived)
- [ ] 2.2 Server: session/ledger schema, lease queue with TTL + heartbeats, at-least-once dedup by first accepted attempt, verdict derivation (`passed`/`failed`/`incomplete`), retention
- [ ] 2.3 Server: `/api/dist/*` endpoints with optional shared-token auth; `/dist` session UI page
- [ ] 2.4 CLI: `lightning worker` (long-poll, checkout at session SHA, run unit with telemetry init script + remote cache, post results; `--drain` mode; unit timeout enforcement)
- [ ] 2.5 CLI: `lightning test-dist` orchestrator (selection reuse, session create, progress polling, exit codes 5/6)
- [ ] 2.6 Server-side merge into one flaky-radar run; infra-failed attempt results discarded
- [ ] 2.7 E2E: multi-worker session over fixture repo — worker kill mid-unit → requeue; failing test → `failed` verdict, no re-execution; empty pool → `incomplete` on timeout

## 3. Stage 3 — class-level sharding (future change)

- [ ] 3.1 Planner: split-by-history with remainder-shard-by-exclusion construction; split threshold
- [ ] 3.2 Worker: `--tests` include filters; init-script exclude channel (env-driven `excludeTestsMatching`); "no tests matched" recognized as non-fatal
- [ ] 3.3 Coverage soft check at merge (history classes vs reported classes, advisory UI warning)
- [ ] 3.4 Property test: shard construction totality — every history class exactly once, unknown classes always land in remainder
