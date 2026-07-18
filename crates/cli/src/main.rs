mod junit;
mod meta;

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
        Command::Upload(args) => upload(args),
        Command::InitScript(args) => init_script(args),
    };
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
