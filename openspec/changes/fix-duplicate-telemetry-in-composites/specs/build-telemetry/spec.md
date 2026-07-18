## MODIFIED Requirements

### Requirement: Publishing at build finish
The init script SHALL POST the collected payload as one JSON document to `<server>/api/builds` when the build finishes. The server URL SHALL be resolved from the Gradle property `lightning.url`, else the environment variable `LIGHTNING_URL`. Each build invocation SHALL carry a unique `build_key` so that re-posting the same payload is idempotent while two separate builds are always two entries. In a composite build the plugin SHALL report from the root build only (`gradle.parent == null`); child builds of the composite SHALL not produce documents, so one invocation yields exactly one document.

#### Scenario: Composite invocation reports once
- **WHEN** a build with included builds runs one invocation with the telemetry init script
- **THEN** exactly one build document is posted, covering the invocation's executed tasks
