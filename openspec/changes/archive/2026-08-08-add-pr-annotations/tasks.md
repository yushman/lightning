## 1. Dependency and scaffolding

- [x] 1.1 Add `rustls` to `ureq`'s feature set in `crates/cli/Cargo.toml`; run `cargo build` to confirm `Cargo.lock` picks up a TLS backend
- [x] 1.2 Create `crates/cli/src/github.rs` and register it in `main.rs`

## 2. PR context resolution

- [x] 2.1 In `github.rs`, implement `PrContext::resolve()`: reads `GITHUB_REPOSITORY`, `GITHUB_TOKEN`, checks `GITHUB_EVENT_NAME == "pull_request"`, reads `GITHUB_EVENT_PATH` JSON for `pull_request.number`; returns `None` if any step fails
- [x] 2.2 Unit tests: full context present → `Some`; non-`pull_request` event → `None`; missing token → `None`; missing/unreadable event file → `None`

## 3. Comment upsert

- [x] 3.1 Implement `list_comments(ctx: &PrContext) -> Result<Vec<Comment>, String>`: `GET /repos/{repo}/issues/{pr}/comments?per_page=100`, follows `Link: rel="next"` headers until exhausted, with a bounded request timeout and no retries (`ureq` timeout config) on every page fetch
- [x] 3.2 Implement `upsert_comment(ctx: &PrContext, marker: &str, body: &str) -> Result<(), String>`: calls `list_comments`, finds first body containing `marker`, `PATCH`es it if found else `POST`s a new comment with `marker` prefixed as an HTML comment; the `PATCH`/`POST` calls also use the bounded timeout
- [x] 3.3 Implement `post_pr_comment(ctx: Option<&PrContext>, marker: &str, body: &str)`: no-ops silently when `ctx` is `None`; on `upsert_comment` error (including timeout), prints a `warning:` line to stderr and returns without propagating the error
- [x] 3.4 Add a Markdown-table cell escaper (`\|` for `|`) used wherever a task path or test name is interpolated into a table cell
- [x] 3.5 Unit/integration tests against a mock `TcpListener`-based HTTP server covering: create-when-absent, patch-when-present, marker comment on a second page (`Link: rel="next"` followed), and fail-safe no-propagate behavior on a non-2xx response. A literal-timeout-firing test was not added (would need ~10s of real wall-clock sleep to exercise the configured timeout, or a config-injection refactor to shorten it for tests); the timeout wiring itself (`agent()` using `timeout_global`) is covered by reading, not by a dedicated test.

## 4. `affected --pr-comment`

- [x] 4.1 Add `--pr-comment` flag to `AffectedArgs`
- [x] 4.2 After selection, when the flag is set, build the Markdown body (module count/list, or everything-affected reason) and call `post_pr_comment` with marker `<!-- lightning:affected -->`
- [x] 4.3 Test: concrete selection produces a body listing count and modules; everything-affected produces a body stating the reason

## 5. `upload --pr-comment`

- [x] 5.1 Add `--pr-comment` flag to `UploadArgs`
- [x] 5.2 After a successful upload, when the flag is set, `GET <server>/api/flaky`, filter to tests present in this run's `results` with `score > 0`, build the Markdown body (table with score + `<server>/tests/<id>` link, or an explicit "no flaky tests" line), call `post_pr_comment` with marker `<!-- lightning:flaky -->`
- [x] 5.3 Test: run containing a currently-flaky test produces a body listing it; run with no overlap with `/api/flaky` produces the explicit empty-state body

## 6. Docs and wiring

- [x] 6.1 Document `--pr-comment` on both commands in `--help` text (clap doc comments), stating the required `GITHUB_TOKEN` permission (`pull-requests: write`)
- [x] 6.2 Add a short section to `README.md` (and `README_RU.md` if that file is kept in sync) showing the workflow YAML snippet: `permissions: pull-requests: write` plus the two flags
- [x] 6.3 Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` for the workspace
