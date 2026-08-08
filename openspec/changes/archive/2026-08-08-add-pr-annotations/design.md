## Context

`lightning affected` and `lightning upload` are both plain CLI subcommands run from CI steps; neither talks to anything but the lightning server (`upload`) or nothing at all (`affected`, which is server-less by design — see `affected-selection`). `crates/cli/Cargo.toml` currently pins `ureq` to `default-features = false, features = ["json"]`: no TLS backend, because the only HTTP call today (`upload` → `<server>/api/runs`) is typically plain HTTP to a self-hosted box on the local network. `api.github.com` requires HTTPS, so reaching it needs a TLS backend added to `ureq`.

## Goals / Non-Goals

**Goals:**
- Surface flaky-score and affected-scope data already computed elsewhere directly on the PR, opt-in via a flag.
- Keep both commands' non-GitHub behavior (exit codes, stdout, `--quiet` semantics for `affected`) byte-for-byte unchanged when `--pr-comment` is not passed.
- Never let a GitHub API problem fail a CI step that would otherwise have succeeded.

**Non-Goals:**
- GitHub Checks API (requires a GitHub App installation, not just a repo token) — plain issue comments work with the default `GITHUB_TOKEN` and `pull-requests: write` permission, which is a one-line workflow change instead of an app to install and maintain.
- GitLab/other CI providers. The PR-context resolution is GitHub Actions-specific; a differently-shaped context resolver for another provider is a separate change if ever needed.
- Combining the two comments into one. Each command upserts its own marker-tagged comment independently, matching the existing "each step is independent, each is removable in a minute" design of the CLI (a user can add `--pr-comment` to `affected` without touching `upload`, or vice versa).

## Decisions

- **Issue comments, not Checks API**: a Check Run requires a GitHub App with a private key and installation flow; an issue comment needs only `GITHUB_TOKEN` (already present in every Actions job) plus `pull-requests: write` permission. Given lightning's one-binary, no-hosted-service posture, avoiding an App install is the right trade — it costs a slightly less structured PR annotation (a comment instead of a check with a status icon), not a functional gap for a reviewer.
- **Two comments, two markers, no shared state**: `--pr-comment` on `affected` and on `upload` run at different points in a job (or in different jobs entirely) and know nothing of each other. Rather than have one invent a combined-comment protocol, each upserts independently under its own HTML-comment marker (`<!-- lightning:affected -->`, `<!-- lightning:flaky -->`). A repo that only wants one signal only turns on one flag.
- **`ureq` gains `rustls`, not `native-tls`**: `rustls` is a pure-Rust TLS stack — no OpenSSL system dependency, consistent with the project's "one static binary" story (musl builds already ship in CI; `native-tls` would fight that on Linux).
- **PR number from `GITHUB_EVENT_PATH`, not the GitHub API**: the event payload is already on disk in every Actions job (`${{ github.event_path }}`) and contains `pull_request.number` directly — one file read, no extra API round-trip, and it's `None` naturally on non-`pull_request` events (the file exists but lacks a `pull_request` key), which doubles as the "are we in a PR" check.
- **Silent no-op outside PR context, loud warning on failure inside it**: matches the existing telemetry init script's fail-safe contract (`build-telemetry` spec: "any error ... shall be caught, logged as a warning, and swallowed"). Silent-outside-PR avoids spamming warnings on every push-to-main build where `--pr-comment` is harmlessly a no-op by design, not a failure.
- **Comment body is plain Markdown, not a template engine**: bodies are short (a count, a list, a table of ≤ tens of rows) — `format!` is enough, no need for a templating dependency.
- **Pagination is followed, not assumed away**: GitHub's list-issue-comments endpoint paginates (default 30/page); an active PR can easily exceed that. `upsert_comment` requests `per_page=100` and follows `Link: rel="next"` until exhausted before deciding the marker is absent — skipping this would silently duplicate the comment on any PR with enough prior discussion, which defeats the entire point of upserting.
- **Bounded timeout, single attempt, no retry**: same rule `build-telemetry` already imposes on the init script's upload ("HTTP requests SHALL have bounded timeouts and a single attempt"). A GitHub API call is no different from a telemetry POST in this respect — either can hang, and `--pr-comment` must never be the reason a CI step times out.

## Risks / Trade-offs

- [Risk] `GITHUB_TOKEN`'s default permissions in some org-level workflow policies are read-only, so `POST`/`PATCH` on comments returns 403. → Mitigation: this is a fail-safe no-op per the design (warning on stderr, exit code untouched); README/flag `--help` text states the required `pull-requests: write` permission explicitly so the fix is a one-line workflow diff.
- [Risk] Two markers means two separate comments cluttering the PR conversation if both flags are used. → Mitigation: accepted trade for independence; each comment is upserted (not appended), so the clutter is at most one comment per marker, not one per run.
- [Risk] `never_cached`-style content growth: an `affected` list on a huge monorepo change could produce a very long comment. → Mitigation: out of scope for this change; GitHub truncates comment bodies at 65536 characters and returns an error past that, which surfaces as the existing fail-safe warning — acceptable for v1, revisit if it's hit in practice.
- [Risk] A module path or test name containing `|` or a backtick breaks a Markdown table's layout in the rendered comment. → Mitigation: not a security issue (GitHub sanitizes comment HTML), but the comment builder escapes `|` as `\|` in table cells so a pathological module/test name can't visually break the table.

## Open Questions

- None blocking. Whether to eventually add a combined single-comment mode is left for a future change if users ask for it.
