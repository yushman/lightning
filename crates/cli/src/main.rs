mod affected;
mod config;
mod gitdiff;
mod github;
mod junit;
mod lock;
mod meta;
mod paths;
mod run;
mod sync;

use clap::{Args, Parser, Subcommand};

use junit::TestResult;

#[derive(Parser)]
#[command(name = "lightning", version, about = "Gradle CI observability toolkit")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse JUnit XML reports and upload a test run to the lightning server
    Upload(UploadArgs),
    /// Emit the embedded Gradle telemetry init script
    InitScript(InitScriptArgs),
    /// Snapshot the Gradle module graph into lightning.lock (runs Gradle once)
    Sync,
    /// List modules affected by the diff against the base ref (no JVM needed)
    Affected(affected::AffectedArgs),
    /// Run a Gradle task on affected modules only
    Run(run::RunArgs),
}

#[derive(Args)]
struct InitScriptArgs {
    /// Write the script to this path instead of stdout
    #[arg(long)]
    out: Option<std::path::PathBuf>,
}

#[derive(Args)]
struct UploadArgs {
    /// Base URL of the lightning server, e.g. http://localhost:8080
    #[arg(long, env = "LIGHTNING_SERVER")]
    server: String,
    /// Glob for JUnit XML reports, relative to the working directory
    #[arg(long, default_value = "**/build/test-results/**/*.xml")]
    glob: String,
    /// Commit sha (default: GITHUB_SHA or git rev-parse HEAD)
    #[arg(long)]
    sha: Option<String>,
    /// Branch (default: GITHUB_REF_NAME or current git branch)
    #[arg(long)]
    branch: Option<String>,
    /// Idempotency key for this run (default: derived from CI env or payload)
    #[arg(long)]
    run_key: Option<String>,
    /// Upsert a PR comment listing tests from this run that are currently
    /// flaky (GitHub Actions pull_request events only, needs GITHUB_TOKEN
    /// with pull-requests: write; silently does nothing otherwise)
    #[arg(long)]
    pr_comment: bool,
}

#[derive(serde::Serialize)]
struct RunPayload<'a> {
    run_key: String,
    sha: String,
    branch: String,
    ci_url: Option<String>,
    results: &'a [TestResult],
}

const INIT_SCRIPT: &str = include_str!("../assets/lightning.init.gradle");

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Upload(args) => upload(args).map(|()| 0),
        Command::InitScript(args) => init_script(args).map(|()| 0),
        Command::Sync => sync_cmd(),
        Command::Affected(args) => cwd().and_then(|dir| affected::run(&dir, &args)),
        Command::Run(args) => cwd().and_then(|dir| run::run(&dir, &args)),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

fn cwd() -> Result<std::path::PathBuf, String> {
    std::env::current_dir().map_err(|e| format!("cannot determine working directory: {e}"))
}

fn sync_cmd() -> Result<i32, String> {
    let dir = cwd()?;
    let cfg = config::load(&dir)?.affected;
    sync::run(&dir, &cfg.invalidate_on).map(|_| 0)
}

fn init_script(args: InitScriptArgs) -> Result<(), String> {
    match args.out {
        Some(path) => {
            std::fs::write(&path, INIT_SCRIPT)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            println!("wrote init script to {}", path.display());
        }
        None => print!("{INIT_SCRIPT}"),
    }
    Ok(())
}

fn collect_results(pattern: &str) -> Result<Vec<TestResult>, String> {
    let paths = glob::glob(pattern).map_err(|e| format!("invalid glob {pattern:?}: {e}"))?;
    let mut results = Vec::new();
    let mut files = 0usize;
    for path in paths.flatten() {
        let xml = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        results.extend(
            junit::parse(&xml).map_err(|e| format!("cannot parse {}: {e}", path.display()))?,
        );
        files += 1;
    }
    if files == 0 {
        return Err(format!("no JUnit XML reports match glob {pattern:?}"));
    }
    Ok(results)
}

fn upload(args: UploadArgs) -> Result<(), String> {
    let results = collect_results(&args.glob)?;
    let meta = meta::resolve(args.sha, args.branch)?;
    let payload = RunPayload {
        run_key: meta::run_key(args.run_key, &meta, &results),
        sha: meta.sha,
        branch: meta.branch,
        ci_url: meta.ci_url,
        results: &results,
    };
    let url = format!("{}/api/runs", args.server.trim_end_matches('/'));
    let mut response = ureq::post(&url)
        .send_json(&payload)
        .map_err(|e| format!("upload to {url} failed: {e}"))?;
    let body: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("invalid server response: {e}"))?;
    let run_id = body["run_id"].as_i64().unwrap_or(0);
    let deduplicated = body["deduplicated"].as_bool().unwrap_or(false);
    if deduplicated {
        println!(
            "run already uploaded (run {run_id}, key {})",
            payload.run_key
        );
    } else {
        println!("uploaded {} results as run {run_id}", results.len());
    }
    if args.pr_comment {
        post_flaky_pr_comment(&args.server, &results);
    }
    Ok(())
}

const FLAKY_PR_COMMENT_MARKER: &str = "<!-- lightning:flaky -->";

fn post_flaky_pr_comment(server: &str, results: &[TestResult]) {
    let Some(ctx) = github::PrContext::resolve() else {
        return;
    };
    match flaky_comment_body(server, results) {
        Ok(body) => github::post_pr_comment(Some(&ctx), FLAKY_PR_COMMENT_MARKER, &body),
        Err(e) => eprintln!("warning: --pr-comment failed: {e}"),
    }
}

/// Fetches `<server>/api/flaky` and builds the comment body for tests from
/// this run that are currently flaky (`score > 0`). Identity match against
/// the run's results is a trimmed exact match on class name and test name —
/// a weak signal, same trade-off as the server's own `never_cached_tasks`.
fn flaky_comment_body(server: &str, results: &[TestResult]) -> Result<String, String> {
    let url = format!("{}/api/flaky", server.trim_end_matches('/'));
    let mut response = github::agent()
        .get(&url)
        .call()
        .map_err(|e| format!("GET {url} failed: {e}"))?;
    let flaky: Vec<serde_json::Value> = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("invalid response from {url}: {e}"))?;
    let present_in_run = |class_name: &str, name: &str| {
        results
            .iter()
            .any(|r| r.class_name.trim() == class_name.trim() && r.name.trim() == name.trim())
    };
    let matches: Vec<&serde_json::Value> = flaky
        .iter()
        .filter(|f| {
            f["score"].as_i64().unwrap_or(0) > 0
                && present_in_run(
                    f["class_name"].as_str().unwrap_or(""),
                    f["name"].as_str().unwrap_or(""),
                )
        })
        .collect();
    if matches.is_empty() {
        return Ok(
            "⚡ **lightning flaky check**: no flaky tests observed in this run.".to_string(),
        );
    }
    let server = server.trim_end_matches('/');
    let rows: String = matches
        .iter()
        .map(|f| {
            let test_id = f["test_id"].as_i64().unwrap_or(0);
            let class_name = github::escape_cell(f["class_name"].as_str().unwrap_or(""));
            let name = github::escape_cell(f["name"].as_str().unwrap_or(""));
            let score = f["score"].as_i64().unwrap_or(0);
            format!("| `{class_name}` `{name}` | {score} | [details]({server}/tests/{test_id}) |\n")
        })
        .collect();
    Ok(format!(
        "⚡ **lightning flaky check**: {} flaky test(s) in this run\n\n\
| Test | Score | |\n|---|---|---|\n{rows}",
        matches.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use junit::Status;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    #[test]
    fn init_script_writes_embedded_asset() {
        let path =
            std::env::temp_dir().join(format!("lightning-init-{}.gradle", std::process::id()));
        init_script(InitScriptArgs {
            out: Some(path.clone()),
        })
        .unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(written, INIT_SCRIPT);
        assert!(written.contains("LightningTelemetryService"));
        assert!(written.contains("/api/builds"));
    }

    fn result(class_name: &str, name: &str) -> TestResult {
        TestResult {
            class_name: class_name.into(),
            name: name.into(),
            status: Status::Pass,
            time_ms: 1,
            message: None,
        }
    }

    /// One-shot mock server for a single `GET /api/flaky` response.
    fn serve_flaky(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n != 0, "connection closed before a full request arrived");
                data.extend_from_slice(&buf[..n]);
                if data.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{addr}")
    }

    #[test]
    fn flaky_comment_body_lists_matching_tests() {
        let server = serve_flaky(
            r#"[{"test_id":1,"class_name":"com.Foo","name":"bar","score":40},
                {"test_id":2,"class_name":"com.Other","name":"baz","score":30}]"#,
        );
        let results = vec![result("com.Foo", "bar")];
        let body = flaky_comment_body(&server, &results).unwrap();
        assert!(body.contains("1 flaky test(s)"));
        assert!(body.contains("com.Foo"));
        assert!(body.contains("bar"));
        assert!(!body.contains("com.Other"));
    }

    #[test]
    fn flaky_comment_body_states_no_flaky_tests_explicitly() {
        let server =
            serve_flaky(r#"[{"test_id":1,"class_name":"com.Foo","name":"bar","score":40}]"#);
        let results = vec![result("com.Unrelated", "test")];
        let body = flaky_comment_body(&server, &results).unwrap();
        assert!(body.contains("no flaky tests observed"));
    }
}
