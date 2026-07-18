# Tasks

- [x] 1. Gate the telemetry plugin on `gradle.parent == null` in lightning.init.gradle
- [x] 2. E2E on detekt: fresh db, one invocation with a cold configuration (no configuration cache reuse) → exactly one build document
- [x] 3. Gates: fmt, clippy, cargo test --workspace

## E2E record (detekt @ 7289f50e, Gradle 9.1, JDK 21)

Before: one `./gradlew :detekt-tooling:test` invocation with the telemetry init script → 3 identical build documents (root + build-logic + detekt-gradle-plugin each received the whole invocation's 42 task events and posted a full report).

After: fresh db, `--no-configuration-cache --rerun-tasks`, one invocation → exactly 1 document with all 42 tasks. Debug run confirmed `gradle.parent` is null only for the root build and `build ':'` for both included builds. Gates: fmt --check, clippy -D warnings, 31+2+23 tests green.
