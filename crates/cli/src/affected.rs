use std::collections::BTreeSet;
use std::path::Path;

use crate::lock::{Lock, Module, Reason};
use crate::{config, gitdiff, lock, paths, sync};

/// Missing/stale lock aborts with this code unless --auto-sync (design D8).
pub const EXIT_STALE: i32 = 4;
/// `--quiet` exit when nothing is affected.
pub const EXIT_NOTHING: i32 = 3;

#[derive(clap::Args)]
pub struct SelectArgs {
    /// Base ref to diff against via merge-base (default: origin/main or
    /// [affected] base in lightning.toml)
    #[arg(long)]
    pub base: Option<String>,
    /// Diff directly against this commit, skipping merge-base resolution
    #[arg(long, conflicts_with = "base")]
    pub base_sha: Option<String>,
    /// Run `lightning sync` automatically when the lock is missing or stale
    #[arg(long)]
    pub auto_sync: bool,
    /// Exclude uncommitted working-tree changes from the diff
    #[arg(long)]
    pub no_uncommitted: bool,
}

pub struct Selection {
    pub base: String,
    pub merge_base: String,
    /// Set when selection degraded to everything-affected, with the reason.
    pub everything: Option<String>,
    /// Affected modules with reasons, sorted by path. When `everything` is
    /// set this contains all modules with `Reason::Everything`.
    pub modules: Vec<(String, Reason)>,
    pub lock: Lock,
}

pub enum Outcome {
    Selected(Selection),
    /// Lock missing or stale without --auto-sync: message + exit code 4.
    Stale(String),
}

pub fn select(dir: &Path, args: &SelectArgs) -> Result<Outcome, String> {
    let cfg = config::load(dir)?.affected;
    let base = args
        .base
        .clone()
        .or_else(|| cfg.base.clone())
        .unwrap_or_else(|| "origin/main".to_string());

    let diff = gitdiff::changed_files(dir, &base, args.base_sha.as_deref(), !args.no_uncommitted)?;

    // opt-in ignore globs, applied before anything else (design D5)
    let ignore: Vec<glob::Pattern> = cfg
        .ignore
        .iter()
        .map(|g| glob::Pattern::new(g).map_err(|e| format!("invalid ignore glob {g:?}: {e}")))
        .collect::<Result<_, _>>()?;
    let files: BTreeSet<String> = diff
        .files
        .into_iter()
        .filter(|f| !ignore.iter().any(|p| p.matches(f)))
        .collect();

    // staleness: recomputed hash mismatch, or the diff touches an
    // invalidation glob (paranoid mode, design D4)
    let mut lock = match Lock::load(dir) {
        Ok(lock) => Some(lock),
        Err(_) if !dir.join(lock::FILE_NAME).exists() => None,
        Err(err) => return Err(err),
    };
    let stale = match &lock {
        None => Some("no lightning.lock found".to_string()),
        Some(l) => {
            let hash = sync::build_files_hash(dir, &cfg.invalidate_on)?;
            let matchers = sync::invalidation_matchers(&cfg.invalidate_on)?;
            if l.build_files_hash != hash {
                Some("lightning.lock is stale (build files changed since sync)".to_string())
            } else {
                files
                    .iter()
                    .find(|f| sync::matches_any(&matchers, f))
                    .map(|f| format!("the diff touches build file {f}"))
            }
        }
    };
    if let Some(reason) = stale {
        if !args.auto_sync {
            return Ok(Outcome::Stale(format!(
                "{reason} — run `lightning sync` (or pass --auto-sync)"
            )));
        }
        lock = Some(sync::run(dir, &cfg.invalidate_on)?);
    }
    let lock = lock.expect("lock present after staleness handling");

    let (everything, modules) = compute(&lock, &files);
    Ok(Outcome::Selected(Selection {
        base,
        merge_base: diff.merge_base,
        everything,
        modules,
        lock,
    }))
}

fn compute(lock: &Lock, files: &BTreeSet<String>) -> (Option<String>, Vec<(String, Reason)>) {
    let all = |reason: String| {
        let modules = lock
            .modules
            .iter()
            .map(|m| (m.path.clone(), Reason::Everything))
            .collect();
        (Some(reason), modules)
    };
    if let Some(reason) = &lock.unsupported {
        return all(reason.clone());
    }
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for file in files {
        match map_file(&lock.modules, file) {
            Some(owners) => changed.extend(owners),
            None => return all(format!("changed file {file} is outside all modules")),
        }
    }
    let affected = lock::closure(&lock.modules, &changed);
    (None, affected.into_iter().collect())
}

/// Declared source dirs first (all matches), then longest module-dir prefix
/// (the root project's `.` never prefix-matches), else None → everything.
fn map_file(modules: &[Module], file: &str) -> Option<Vec<String>> {
    let by_source: Vec<String> = modules
        .iter()
        .filter(|m| m.source_dirs.iter().any(|sd| paths::is_under(file, sd)))
        .map(|m| m.path.clone())
        .collect();
    if !by_source.is_empty() {
        return Some(by_source);
    }
    modules
        .iter()
        .filter(|m| m.dir != "." && paths::is_under(file, &m.dir))
        .max_by_key(|m| m.dir.len())
        .map(|m| vec![m.path.clone()])
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Text,
    Json,
    GithubMatrix,
}

#[derive(clap::Args)]
pub struct AffectedArgs {
    #[command(flatten)]
    pub select: SelectArgs,
    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    pub format: Format,
    /// Shorthand for --format json
    #[arg(long, conflicts_with = "format")]
    pub json: bool,
    /// No output; exit 0 when something is affected, 3 when nothing is
    #[arg(long, conflicts_with_all = ["format", "json"])]
    pub quiet: bool,
}

pub fn run(dir: &Path, args: &AffectedArgs) -> Result<i32, String> {
    let selection = match select(dir, &args.select)? {
        Outcome::Selected(s) => s,
        Outcome::Stale(message) => {
            eprintln!("error: {message}");
            return Ok(EXIT_STALE);
        }
    };
    if let Some(reason) = &selection.everything {
        eprintln!("lightning: warning: {reason}; selecting everything");
    }
    if args.quiet {
        return Ok(if selection.modules.is_empty() {
            EXIT_NOTHING
        } else {
            0
        });
    }
    let format = if args.json { Format::Json } else { args.format };
    match format {
        Format::Text => {
            for (path, _) in &selection.modules {
                println!("{path}");
            }
        }
        Format::Json => {
            let modules: Vec<serde_json::Value> = selection
                .modules
                .iter()
                .map(|(path, reason)| serde_json::json!({ "path": path, "reason": reason }))
                .collect();
            let out = serde_json::json!({
                "base": selection.base,
                "merge_base": selection.merge_base,
                "everything": selection.everything.is_some(),
                "reason": selection.everything,
                "modules": modules,
            });
            println!("{}", serde_json::to_string_pretty(&out).expect("json"));
        }
        Format::GithubMatrix => {
            let include: Vec<serde_json::Value> = selection
                .modules
                .iter()
                .map(|(path, _)| serde_json::json!({ "module": path }))
                .collect();
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({ "include": include })).expect("json")
            );
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::{Dep, EdgeKind};

    fn lock() -> Lock {
        let module =
            |path: &str, dir: &str, source_dirs: &[&str], deps: &[(&str, EdgeKind)]| Module {
                path: path.into(),
                dir: dir.into(),
                source_dirs: source_dirs.iter().map(|s| (*s).into()).collect(),
                tasks: vec![],
                deps: deps
                    .iter()
                    .map(|(p, k)| Dep {
                        path: (*p).into(),
                        kind: *k,
                    })
                    .collect(),
            };
        Lock {
            version: 1,
            build_files_hash: "h".into(),
            unsupported: None,
            modules: vec![
                module(":", ".", &[], &[]),
                module(":core", "core", &["core/src/main/java", "shared/src"], &[]),
                module(
                    ":lib",
                    "lib",
                    &["lib/src/main/java"],
                    &[(":core", EdgeKind::Main)],
                ),
                module(
                    ":app",
                    "app",
                    &["app/src/main/java"],
                    &[(":lib", EdgeKind::Main), (":fixtures", EdgeKind::Test)],
                ),
                module(":fixtures", "fixtures", &["fixtures/src/main/java"], &[]),
            ],
        }
    }

    fn files(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| (*p).into()).collect()
    }

    #[test]
    fn maps_via_out_of_module_source_dir() {
        let (everything, modules) = compute(&lock(), &files(&["shared/src/S.java"]));
        assert!(everything.is_none());
        let paths: Vec<&str> = modules.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec![":app", ":core", ":lib"]);
    }

    #[test]
    fn dir_prefix_maps_non_source_files() {
        let (everything, modules) = compute(&lock(), &files(&["fixtures/build.gradle"]));
        assert!(everything.is_none());
        let paths: Vec<&str> = modules.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec![":app", ":fixtures"]);
        assert_eq!(modules[0].1, Reason::TestDependency);
    }

    #[test]
    fn outside_file_degrades_to_everything() {
        for outside in ["ci-config.yml", "../above-root.md"] {
            let (everything, modules) = compute(&lock(), &files(&[outside]));
            assert!(everything.is_some(), "{outside} should degrade");
            assert_eq!(modules.len(), lock().modules.len());
            assert!(modules.iter().all(|(_, r)| *r == Reason::Everything));
        }
    }

    #[test]
    fn root_module_reachable_only_via_source_dirs() {
        let mut l = lock();
        l.modules[0].source_dirs = vec!["src/main/java".into()];
        let (everything, modules) = compute(&l, &files(&["src/main/java/R.java"]));
        assert!(everything.is_none());
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].0, ":");
    }

    #[test]
    fn unsupported_lock_selects_everything() {
        let mut l = lock();
        l.unsupported = Some("composite build".into());
        let (everything, modules) = compute(&l, &files(&["core/src/main/java/C.java"]));
        assert_eq!(everything.as_deref(), Some("composite build"));
        assert_eq!(modules.len(), l.modules.len());
    }

    #[test]
    fn empty_diff_affects_nothing() {
        let (everything, modules) = compute(&lock(), &BTreeSet::new());
        assert!(everything.is_none());
        assert!(modules.is_empty());
    }
}
