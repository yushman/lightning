use std::process::Command;

use crate::junit::TestResult;

pub struct RunMeta {
    pub sha: String,
    pub branch: String,
    pub ci_url: Option<String>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn resolve(sha: Option<String>, branch: Option<String>) -> Result<RunMeta, String> {
    let sha = sha
        .or_else(|| env("GITHUB_SHA"))
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .ok_or("cannot determine commit sha: pass --sha, set GITHUB_SHA, or run in a git repo")?;
    let branch = branch
        .or_else(|| env("GITHUB_HEAD_REF"))
        .or_else(|| env("GITHUB_REF_NAME"))
        .or_else(|| git(&["rev-parse", "--abbrev-ref", "HEAD"]))
        .ok_or(
            "cannot determine branch: pass --branch, set GITHUB_REF_NAME, or run in a git repo",
        )?;
    let ci_url = match (
        env("GITHUB_SERVER_URL"),
        env("GITHUB_REPOSITORY"),
        env("GITHUB_RUN_ID"),
    ) {
        (Some(server), Some(repo), Some(run_id)) => {
            Some(format!("{server}/{repo}/actions/runs/{run_id}"))
        }
        _ => None,
    };
    Ok(RunMeta {
        sha,
        branch,
        ci_url,
    })
}

pub fn run_key(explicit: Option<String>, meta: &RunMeta, results: &[TestResult]) -> String {
    if let Some(key) = explicit {
        return key;
    }
    if let (Some(repo), Some(run_id)) = (env("GITHUB_REPOSITORY"), env("GITHUB_RUN_ID")) {
        let attempt = env("GITHUB_RUN_ATTEMPT").unwrap_or_else(|| "1".into());
        return format!("gh:{repo}:{run_id}:{attempt}");
    }
    format!("local:{}", results_digest(&meta.sha, &meta.branch, results))
}

fn results_digest(sha: &str, branch: &str, results: &[TestResult]) -> String {
    let mut lines: Vec<String> = results
        .iter()
        .map(|r| {
            format!(
                "{}\x00{}\x00{}",
                r.class_name,
                r.name,
                serde_json::to_string(&r.status).unwrap()
            )
        })
        .collect();
    lines.sort();
    let mut hasher = blake3::Hasher::new();
    hasher.update(sha.as_bytes());
    hasher.update(b"\x00");
    hasher.update(branch.as_bytes());
    for line in &lines {
        hasher.update(b"\x00");
        hasher.update(line.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::junit::Status;

    fn result(name: &str, status: Status) -> TestResult {
        TestResult {
            class_name: "C".into(),
            name: name.into(),
            status,
            time_ms: 1,
            message: None,
        }
    }

    #[test]
    fn digest_is_order_independent_and_status_sensitive() {
        let a = vec![result("a", Status::Pass), result("b", Status::Fail)];
        let b = vec![result("b", Status::Fail), result("a", Status::Pass)];
        let c = vec![result("a", Status::Fail), result("b", Status::Fail)];
        assert_eq!(
            results_digest("s", "main", &a),
            results_digest("s", "main", &b)
        );
        assert_ne!(
            results_digest("s", "main", &a),
            results_digest("s", "main", &c)
        );
        assert_ne!(
            results_digest("s", "main", &a),
            results_digest("s2", "main", &a)
        );
    }
}
