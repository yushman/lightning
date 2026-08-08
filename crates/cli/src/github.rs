use std::time::Duration;

const API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "lightning-cli";
const TIMEOUT: Duration = Duration::from_secs(10);

/// PR-comment context resolved from GitHub Actions environment. Absent
/// (rather than an error) whenever `--pr-comment` should silently no-op:
/// not a `pull_request` event, no token, or an unreadable event payload.
pub struct PrContext {
    repo: String,
    pr_number: u64,
    token: String,
}

impl PrContext {
    pub fn resolve() -> Option<PrContext> {
        if env("GITHUB_EVENT_NAME").as_deref() != Some("pull_request") {
            return None;
        }
        let repo = env("GITHUB_REPOSITORY")?;
        let token = env("GITHUB_TOKEN")?;
        let event_path = env("GITHUB_EVENT_PATH")?;
        let contents = std::fs::read_to_string(event_path).ok()?;
        let event: serde_json::Value = serde_json::from_str(&contents).ok()?;
        let pr_number = event.get("pull_request")?.get("number")?.as_u64()?;
        Some(PrContext {
            repo,
            pr_number,
            token,
        })
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// A bounded-timeout, single-attempt agent — used for GitHub API calls, and
/// reusable by any other CLI HTTP call that wants the same fail-safe posture.
pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into()
}

#[derive(serde::Deserialize)]
struct Comment {
    id: i64,
    body: String,
}

/// Escapes `|` so a task path or test name can't break a Markdown table cell.
pub fn escape_cell(s: &str) -> String {
    s.replace('|', r"\|")
}

fn next_page_url(response: &ureq::http::Response<ureq::Body>) -> Option<String> {
    let link = response.headers().get("link")?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let (url_part, rel_part) = part.split_once(';')?;
        if rel_part.trim() != r#"rel="next""# {
            return None;
        }
        Some(
            url_part
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string(),
        )
    })
}

fn list_comments(agent: &ureq::Agent, ctx: &PrContext, base: &str) -> Result<Vec<Comment>, String> {
    let mut all = Vec::new();
    let mut url = format!(
        "{base}/repos/{}/issues/{}/comments?per_page=100",
        ctx.repo, ctx.pr_number
    );
    loop {
        let mut response = agent
            .get(&url)
            .header("Authorization", format!("Bearer {}", ctx.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("GitHub API request to {url} failed: {e}"))?;
        let next = next_page_url(&response);
        let page: Vec<Comment> = response
            .body_mut()
            .read_json()
            .map_err(|e| format!("invalid GitHub API response from {url}: {e}"))?;
        all.extend(page);
        match next {
            Some(u) => url = u,
            None => break,
        }
    }
    Ok(all)
}

/// Lists existing PR comments (following pagination), patches the first whose
/// body contains `marker` if found, else creates a new one with `marker`
/// (an HTML comment, e.g. `<!-- lightning:affected -->`) prefixed. Re-running
/// with the same marker always updates the same comment.
fn upsert_comment_at(base: &str, ctx: &PrContext, marker: &str, body: &str) -> Result<(), String> {
    let agent = agent();
    let existing = list_comments(&agent, ctx, base)?
        .into_iter()
        .find(|c| c.body.contains(marker));
    let full_body = format!("{marker}\n{body}");
    match existing {
        Some(comment) => {
            let url = format!("{base}/repos/{}/issues/comments/{}", ctx.repo, comment.id);
            agent
                .patch(&url)
                .header("Authorization", format!("Bearer {}", ctx.token))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", USER_AGENT)
                .send_json(serde_json::json!({ "body": full_body }))
                .map_err(|e| format!("GitHub API request to {url} failed: {e}"))?;
        }
        None => {
            let url = format!(
                "{base}/repos/{}/issues/{}/comments",
                ctx.repo, ctx.pr_number
            );
            agent
                .post(&url)
                .header("Authorization", format!("Bearer {}", ctx.token))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", USER_AGENT)
                .send_json(serde_json::json!({ "body": full_body }))
                .map_err(|e| format!("GitHub API request to {url} failed: {e}"))?;
        }
    }
    Ok(())
}

fn upsert_comment(ctx: &PrContext, marker: &str, body: &str) -> Result<(), String> {
    upsert_comment_at(API_BASE, ctx, marker, body)
}

/// Fail-safe entry point: no-ops silently when `ctx` is `None`; on any
/// `upsert_comment` error, prints a warning to stderr and returns without
/// propagating the error, so `--pr-comment` can never fail the command.
pub fn post_pr_comment(ctx: Option<&PrContext>, marker: &str, body: &str) {
    let Some(ctx) = ctx else {
        return;
    };
    if let Err(e) = upsert_comment(ctx, marker, body) {
        eprintln!("warning: --pr-comment failed: {e}");
    }
}

#[cfg(test)]
impl PrContext {
    fn for_test(repo: &str, pr_number: u64, token: &str) -> PrContext {
        PrContext {
            repo: repo.into(),
            pr_number,
            token: token.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// Reads a full HTTP/1.1 request (headers + `Content-Length` body, if
    /// any) off `stream`. A single `read()` call can return only part of
    /// what the client sent, so this keeps reading until it has it all.
    pub(crate) fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n != 0, "connection closed before a full request arrived");
            data.extend_from_slice(&buf[..n]);
            let Some(header_end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&data[..header_end]);
            let content_length: usize = headers
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim())
                })
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if data.len() >= header_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    struct MockResponse {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: String,
    }

    fn ok_json(body: &str) -> MockResponse {
        MockResponse {
            status: 200,
            headers: Vec::new(),
            body: body.to_string(),
        }
    }

    /// Serves each response in order on its own connection (`Connection:
    /// close`, so `ureq` can't pool/reuse across the ones that follow) and
    /// returns the base URL plus a handle yielding the raw request lines seen.
    fn serve(responses: Vec<MockResponse>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for resp in responses {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(read_request(&mut stream));
                let mut raw = format!(
                    "HTTP/1.1 {} X\r\nConnection: close\r\nContent-Length: {}\r\n",
                    resp.status,
                    resp.body.len()
                );
                for (k, v) in &resp.headers {
                    raw.push_str(&format!("{k}: {v}\r\n"));
                }
                raw.push_str("\r\n");
                raw.push_str(&resp.body);
                stream.write_all(raw.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
            requests
        });
        (base_url, handle)
    }

    #[test]
    fn creates_comment_when_marker_absent() {
        let (base, handle) = serve(vec![ok_json("[]"), ok_json(r#"{"id":1}"#)]);
        let ctx = PrContext::for_test("o/r", 7, "tok");
        upsert_comment_at(&base, &ctx, "<!-- m -->", "hello").unwrap();
        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("GET /repos/o/r/issues/7/comments"));
        assert!(requests[1].starts_with("POST /repos/o/r/issues/7/comments"));
        assert!(requests[1].contains("hello"));
    }

    #[test]
    fn patches_existing_comment_when_marker_present() {
        let (base, handle) = serve(vec![
            ok_json(r#"[{"id":42,"body":"<!-- m -->\nold"}]"#),
            ok_json(r#"{"id":42}"#),
        ]);
        let ctx = PrContext::for_test("o/r", 7, "tok");
        upsert_comment_at(&base, &ctx, "<!-- m -->", "new").unwrap();
        let requests = handle.join().unwrap();
        assert!(requests[1].starts_with("PATCH /repos/o/r/issues/comments/42"));
        assert!(requests[1].contains("new"));
    }

    #[test]
    fn follows_pagination_to_find_marker_on_a_later_page() {
        // The Link header on page 1 is filled in with page 2's real URL once
        // we know the mock server's port; two channel-like passes avoid that
        // chicken-and-egg by building page 2's response after `serve` binds.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let page2_url = format!("{base}/repos/o/r/issues/7/comments?per_page=100&page=2");
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            let responses = vec![
                MockResponse {
                    status: 200,
                    headers: vec![("Link", format!(r#"<{page2_url}>; rel="next""#))],
                    body: r#"[{"id":1,"body":"unrelated"}]"#.to_string(),
                },
                ok_json(r#"[{"id":99,"body":"<!-- m -->\nold"}]"#),
                ok_json(r#"{"id":99}"#),
            ];
            for resp in responses {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(read_request(&mut stream));
                let mut raw = format!(
                    "HTTP/1.1 {} X\r\nConnection: close\r\nContent-Length: {}\r\n",
                    resp.status,
                    resp.body.len()
                );
                for (k, v) in &resp.headers {
                    raw.push_str(&format!("{k}: {v}\r\n"));
                }
                raw.push_str("\r\n");
                raw.push_str(&resp.body);
                stream.write_all(raw.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
            requests
        });
        let ctx = PrContext::for_test("o/r", 7, "tok");
        upsert_comment_at(&base, &ctx, "<!-- m -->", "new").unwrap();
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].starts_with("PATCH /repos/o/r/issues/comments/99"));
    }

    #[test]
    fn fails_safe_and_does_not_panic_on_non_2xx() {
        let (base, _handle) = serve(vec![MockResponse {
            status: 500,
            headers: Vec::new(),
            body: "boom".to_string(),
        }]);
        let ctx = PrContext::for_test("o/r", 7, "tok");
        // upsert_comment_at surfaces the error to the caller...
        assert!(upsert_comment_at(&base, &ctx, "<!-- m -->", "x").is_err());
        // ...but the public entry point swallows it and never panics.
        post_pr_comment(Some(&ctx), "<!-- m -->", "x");
    }

    #[test]
    fn post_pr_comment_no_ops_without_context() {
        // No server listening at all: if this tried to make a request it
        // would hang or error loudly instead of returning immediately.
        post_pr_comment(None, "<!-- m -->", "x");
    }

    #[test]
    fn escape_cell_escapes_pipe() {
        assert_eq!(escape_cell("a|b"), r"a\|b");
        assert_eq!(escape_cell("plain"), "plain");
    }
}
