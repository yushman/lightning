use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::paths;

pub struct Diff {
    /// Commit actually diffed against (merge-base or explicit sha).
    pub merge_base: String,
    /// Changed files relative to `dir` (the Gradle root); paths outside it
    /// keep leading `..` components.
    pub files: BTreeSet<String>,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn resolve_merge_base(dir: &Path, base: &str) -> Result<String, String> {
    match git(dir, &["merge-base", base, "HEAD"]) {
        Ok(sha) => Ok(sha.trim().to_string()),
        Err(err) => {
            let shallow = git(dir, &["rev-parse", "--is-shallow-repository"])
                .map(|v| v.trim() == "true")
                .unwrap_or(false);
            if shallow {
                return Err(format!(
                    "cannot find merge-base({base}, HEAD) in a shallow clone — fetch full \
                     history (GitHub Actions: `fetch-depth: 0`, or `git fetch --unshallow`), \
                     or pass --base-sha <sha> if CI already knows the base commit"
                ));
            }
            if git(
                dir,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{base}^{{commit}}"),
                ],
            )
            .is_err()
            {
                return Err(format!(
                    "base ref {base:?} is unknown — fetch it (e.g. `git fetch origin main`), \
                     or override with --base / [affected] base in lightning.toml"
                ));
            }
            Err(err)
        }
    }
}

/// Changed files: `git diff --name-only merge-base(base, HEAD)..HEAD`, plus
/// working-tree changes (staged, unstaged, untracked) unless disabled.
/// `base_sha` skips merge-base resolution.
pub fn changed_files(
    dir: &Path,
    base: &str,
    base_sha: Option<&str>,
    include_uncommitted: bool,
) -> Result<Diff, String> {
    let repo_root = git(dir, &["rev-parse", "--show-toplevel"])?;
    let repo_root = Path::new(repo_root.trim());
    let dir_abs = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    let repo_root = repo_root
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", repo_root.display()))?;
    let prefix = dir_abs
        .strip_prefix(&repo_root)
        .map_err(|_| "working directory is not inside the git repository".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let prefix = if prefix.is_empty() {
        ".".to_string()
    } else {
        prefix
    };

    let merge_base = match base_sha {
        Some(sha) => git(
            dir,
            &["rev-parse", "--verify", &format!("{sha}^{{commit}}")],
        )
        .map(|v| v.trim().to_string())
        .map_err(|_| format!("--base-sha {sha:?} is not a known commit"))?,
        None => resolve_merge_base(dir, base)?,
    };

    let mut repo_relative: BTreeSet<String> = BTreeSet::new();
    let diff = git(
        dir,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            &merge_base,
            "HEAD",
        ],
    )?;
    repo_relative.extend(diff.split('\0').filter(|p| !p.is_empty()).map(String::from));

    if include_uncommitted {
        let status = git(
            dir,
            &["status", "--porcelain", "-z", "--untracked-files=all"],
        )?;
        let mut entries = status.split('\0').filter(|e| !e.is_empty());
        while let Some(entry) = entries.next() {
            if entry.len() < 4 {
                continue;
            }
            let (xy, path) = entry.split_at(3);
            repo_relative.insert(path.to_string());
            if xy.starts_with('R') || xy.starts_with('C') {
                // rename/copy: the origin path follows as its own entry
                if let Some(origin) = entries.next() {
                    repo_relative.insert(origin.to_string());
                }
            }
        }
    }

    let files = repo_relative
        .into_iter()
        .map(|p| paths::rebase(&p, &prefix))
        // the CLI's own artifact and config: the lock never maps anywhere,
        // the config participates via the invalidation hash instead
        .filter(|p| p != crate::lock::FILE_NAME && p != crate::config::FILE_NAME)
        .collect();
    Ok(Diff { merge_base, files })
}
