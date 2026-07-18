# design-test-distribution

## Why

Phases 1–4 shipped observability (flaky radar, build telemetry), acceleration (remote build cache), and selection (affected). The remaining CI bottleneck is wall-clock test time: even a perfectly selected, fully cached build still runs its affected tests serially per CI job. Develocity's answer is Test Distribution — a paid, closed feature with no OSS equivalent for Gradle. Distributing test execution across a fleet of workers is the platform's declared north star (phase 5), and it is the phase where all accumulated data compounds: per-test timings (phase 1) and per-task timings (phase 2) drive shard balancing, the remote cache (phase 3) warms workers, and the module graph (phase 4) defines the work units.

Per `docs/design/phases-dod.md`, phase 5 is **design-only**: explore + proposal + design, no implementation, change stays proposed. This change is that design.

## What Changes

Nothing in the codebase. This change records the groomed design for distributed Gradle test execution:

- The **"outside the build" tension resolved explicitly**: work units are whole Gradle invocations (`:module:testTask`, later narrowed by test-class filters), never a replaced test executer. The single sanctioned step inside the build is the already-established init-script channel, used only for class-exclusion filters in the final stage.
- **Architecture**: a scheduler inside `lightning-server` (SQLite-backed session/unit ledger, HTTP long-poll dispatch), a worker mode in the existing CLI (`lightning worker`), and an orchestrator entry point (`lightning test-dist`) that plans units from the phase-4 lock and waits for the merged verdict.
- **Balancing** from phase-1/phase-2 timing history with an explicit cold-start fallback.
- **Failure semantics** with an FN-never analog: an assigned-vs-reported unit ledger — a session can only end `passed`/`failed`/`incomplete`, and a test is never silently dropped.
- **Staged delivery** in three shippable slices (static timing-balanced sharding → dynamic queue with workers → class-level sharding), each valuable on its own.
- **Prerequisites** that phases 1–4 must grow first (notably: module attribution for ingested test results) and open questions.

## Capabilities

### New Capabilities

- `test-distribution`: shard planning, distribution sessions, work-unit ledger and dispatch protocol, worker agent, result merge, balancing, and failure semantics. The delta spec in this change is the **target contract for the future implementation stages**; it lands in `openspec/specs/` only when the corresponding implementation changes are archived — this change is never archived.

### Modified Capabilities

None. Stage 1 will extend the `affected` CLI surface and stage 2 the server ingest path; those deltas belong to the future per-stage implementation changes, not to this design-only change.

## Impact

- No code, schema, endpoint, or existing-spec changes.
- New artifacts under `openspec/changes/design-test-distribution/` only.
- The change remains in proposed state indefinitely (per phase-5 DoD); tasks.md is a deferred implementation outline, not work to execute now.
