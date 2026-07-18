## ADDED Requirements

### Requirement: Cache read endpoint
The server SHALL serve `GET /cache/{key}` returning the stored artifact bytes with `Content-Type: application/octet-stream` and status `200`, or `404` when no entry exists for the key. A successful read SHALL increment the entry's hit count and refresh its last-access time. `HEAD /cache/{key}` SHALL return the same status without a body and without counting a hit.

#### Scenario: Hit
- **WHEN** an artifact was stored under a key and `GET /cache/{key}` is requested
- **THEN** the response is `200` with exactly the stored bytes and the entry's hit count increases

#### Scenario: Miss
- **WHEN** `GET /cache/{key}` is requested for an unknown key
- **THEN** the response is `404`

### Requirement: Cache write endpoint
The server SHALL accept `PUT /cache/{key}` with an opaque binary body, persist it durably (no partially written artifact is ever readable), index the entry in SQLite (key, size, creation time, last-access time, hit count), and respond with a `2xx` status. Re-putting an existing key SHALL overwrite the stored artifact.

#### Scenario: Roundtrip
- **WHEN** a body is `PUT` to `/cache/{key}` and then `GET /cache/{key}` is requested
- **THEN** the `PUT` returns `2xx` and the `GET` returns the identical bytes

### Requirement: Key validation
Cache keys SHALL be validated as lowercase hexadecimal strings of 32 to 64 characters. Requests to `/cache/{key}` with any other key SHALL be rejected with `400` and SHALL NOT touch storage.

#### Scenario: Invalid key rejected
- **WHEN** a key containing uppercase, non-hex characters, or fewer than 32 characters is used in a cache request
- **THEN** the response is `400` and nothing is stored or read

### Requirement: Artifact size limit
The server SHALL reject `PUT` bodies larger than a configurable maximum artifact size (default 100 MiB, `--cache-max-artifact-mb` / `LIGHTNING_CACHE_MAX_ARTIFACT_MB`) with status `413` without storing anything.

#### Scenario: Oversized artifact rejected
- **WHEN** a body larger than the configured maximum is `PUT` to a valid key
- **THEN** the response is `413` and no entry is created

### Requirement: Total size cap with LRU eviction
The server SHALL keep the total indexed cache size at or below a configurable maximum (default 10 GiB, `--cache-max-size-mb` / `LIGHTNING_CACHE_MAX_SIZE_MB`). When a write pushes the total over the cap, the server SHALL evict least-recently-accessed entries (excluding the entry just written) — removing both index row and file — until the total is within the cap.

#### Scenario: LRU entry evicted on write
- **WHEN** the cache is at capacity and a new artifact is written
- **THEN** the least-recently-accessed entries are removed until the total fits and the new entry remains readable

### Requirement: Entry TTL retention
The server SHALL delete cache entries whose last-access time is older than a configurable number of days (default 30, `--cache-retention-days` / `LIGHTNING_CACHE_RETENTION_DAYS`), at startup and on writes.

#### Scenario: Stale entry pruned
- **WHEN** an entry has not been accessed within the retention window and the server starts or a write occurs
- **THEN** the entry and its file are removed

### Requirement: Optional write authentication
When a shared token is configured (`--cache-token` / `LIGHTNING_CACHE_TOKEN`), `PUT /cache/{key}` SHALL require HTTP Basic authentication whose password equals the token (username ignored); requests without it SHALL get `401` with a `WWW-Authenticate: Basic` header. Reads SHALL remain unauthenticated. Without a configured token, writes SHALL be open.

#### Scenario: Write rejected without token
- **WHEN** a token is configured and a `PUT` arrives with no or wrong credentials
- **THEN** the response is `401` and nothing is stored

#### Scenario: Write accepted with token
- **WHEN** a token is configured and a `PUT` carries Basic credentials with the token as password
- **THEN** the artifact is stored and the response is `2xx`

### Requirement: Storage reconciliation at startup
Artifacts SHALL be stored as files in a configurable directory (default `lightning-cache` next to the database file, `--cache-dir` / `LIGHTNING_CACHE_DIR`) indexed by SQLite. At startup the server SHALL reconcile index and filesystem: index rows without a file and files (including temp files) without an index row SHALL be removed.

#### Scenario: Orphan file removed
- **WHEN** a file exists in the cache directory without a corresponding index row and the server starts
- **THEN** the file is deleted and the indexed total reflects only indexed entries
