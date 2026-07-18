# fix-duplicate-telemetry-in-composites

## Why

Dogfooding on detekt (36 modules, two included builds): one `./gradlew :detekt-tooling:test` invocation with the telemetry init script produced **three identical build documents** (same 42 tasks, same duration). Gradle applies init scripts to every build of a composite, and `BuildEventsListenerRegistry.onTaskCompletion` delivers the whole invocation's task events to each registered service — so the root build, `build-logic`, and `detekt-gradle-plugin` each posted a full report. Build counts, trend medians, and cache hit-rate analytics count every composite CI build N times. nowinandroid masked this because its child build was configuration-cached during our earlier E2E.

## What Changes

- The telemetry plugin no-ops in non-root builds (`gradle.parent != null`): exactly one document per invocation.

## Capabilities

### Modified Capabilities

- `build-telemetry`: one build document per Gradle invocation, reported by the root build only.

## Impact

- crates/cli/assets/lightning.init.gradle only. No server or schema change.
