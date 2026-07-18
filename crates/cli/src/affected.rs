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

    // staleness: the recomputed hash is the sole authority — it covers the
    // full invalidation set against the current working tree, so a diff-based
    // check adds no FN protection, only false positives
    let mut lock = match Lock::load(dir) {
        Ok(lock) => Some(lock),
        Err(_) if !dir.join(lock::FILE_NAME).exists() => None,
        Err(err) => return Err(err),
    };
    let stale = match &lock {
        None => Some("no lightning.lock found".to_string()),
        Some(l) => {
            // included-build roots recorded in the lock join the invalidation
            // set (dynamic `<dir>/**` globs), symmetric with sync
            let mut globs = cfg.invalidate_on.clone();
            globs.extend(sync::included_build_globs(&l.included_builds));
            let hash = sync::build_files_hash(dir, &globs)?;
            (l.build_files_hash != hash)
                .then(|| "lightning.lock is stale (build files changed since sync)".to_string())
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
    // everything-affected lists all modules except a bare root project:
    // `:` without declared source dirs is a container (nothing to test) and
    // would only receive meaningless root-level tasks from `run`
    let all = |reason: String| {
        let modules = lock
            .modules
            .iter()
            .filter(|m| m.path != ":" || !m.source_dirs.is_empty())
            .map(|m| (m.path.clone(), Reason::Everything))
            .collect();
        (Some(reason), modules)
    };
    if let Some(reason) = &lock.unsupported {
        return all(reason.clone());
    }
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for file in files {
        // a file inside an included-build root is a build-logic change, not
        // "outside all modules": convention plugins can reconfigure any
        // module, so everything is affected — with an honest reason
        if let Some(dir) = lock
            .included_builds
            .iter()
            .find(|d| paths::is_under(file, d))
        {
            return all(format!(
                "build logic changed: {file} is inside included build {dir}"
            ));
        }
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
            version: crate::lock::VERSION,
            build_files_hash: "h".into(),
            unsupported: None,
            included_builds: vec![],
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
    fn resynced_build_file_change_is_not_stale() {
        let dir = std::env::temp_dir().join(format!("lightning-resync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app/src")).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("app/build.gradle"), "// v1\n").unwrap();
        std::fs::write(dir.join("app/src/A.java"), "class A {}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        let base = {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&dir)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        // build file modified after the base commit, lock synced AFTER that:
        // its hash reflects the current tree, so selection must proceed
        std::fs::write(dir.join("app/build.gradle"), "// v2\n").unwrap();
        std::fs::write(dir.join("app/src/A.java"), "class A { int x; }\n").unwrap();
        let mut l = lock();
        l.modules = vec![Module {
            path: ":app".into(),
            dir: "app".into(),
            source_dirs: vec!["app/src".into()],
            tasks: vec![],
            deps: vec![],
        }];
        l.build_files_hash = sync::build_files_hash(&dir, &[]).unwrap();
        std::fs::write(
            dir.join(lock::FILE_NAME),
            serde_json::to_string(&l).unwrap(),
        )
        .unwrap();
        let args = SelectArgs {
            base: None,
            base_sha: Some(base),
            auto_sync: false,
            no_uncommitted: false,
        };
        match select(&dir, &args).unwrap() {
            Outcome::Selected(sel) => {
                assert!(sel.everything.is_none(), "everything: {:?}", sel.everything);
                assert_eq!(sel.modules.len(), 1);
                assert_eq!(sel.modules[0].0, ":app");
            }
            Outcome::Stale(msg) => panic!("false stale: {msg}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outside_file_degrades_to_everything() {
        for outside in ["ci-config.yml", "../above-root.md"] {
            let (everything, modules) = compute(&lock(), &files(&[outside]));
            assert!(everything.is_some(), "{outside} should degrade");
            // all modules except the bare root project `:`
            assert_eq!(modules.len(), lock().modules.len() - 1);
            assert!(
                modules
                    .iter()
                    .all(|(p, r)| p != ":" && *r == Reason::Everything)
            );
        }
    }

    #[test]
    fn everything_keeps_root_with_declared_source_dirs() {
        let mut l = lock();
        l.modules[0].source_dirs = vec!["src/main/java".into()];
        let (everything, modules) = compute(&l, &files(&["ci-config.yml"]));
        assert!(everything.is_some());
        assert!(modules.iter().any(|(p, _)| p == ":"));
    }

    #[test]
    fn included_build_file_degrades_as_build_logic_change() {
        let mut l = lock();
        l.included_builds = vec!["gradle/plugins".into()];
        let (everything, modules) =
            compute(&l, &files(&["gradle/plugins/src/main/kotlin/Conv.kt"]));
        let reason = everything.expect("degrades");
        assert!(reason.contains("build logic changed"), "{reason}");
        assert!(reason.contains("gradle/plugins"), "{reason}");
        assert!(!reason.contains("outside all modules"), "{reason}");
        assert_eq!(modules.len(), l.modules.len() - 1); // bare root excluded
    }

    #[test]
    fn files_outside_included_builds_map_normally() {
        let mut l = lock();
        l.included_builds = vec!["gradle/plugins".into()];
        let (everything, modules) = compute(&l, &files(&["core/src/main/java/C.java"]));
        assert!(everything.is_none());
        let paths: Vec<&str> = modules.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec![":app", ":core", ":lib"]);
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
        l.unsupported = Some("dependency substitution into included build(s)".into());
        let (everything, modules) = compute(&l, &files(&["core/src/main/java/C.java"]));
        assert_eq!(
            everything.as_deref(),
            Some("dependency substitution into included build(s)")
        );
        assert_eq!(modules.len(), l.modules.len() - 1); // bare root excluded
    }

    #[test]
    fn empty_diff_affects_nothing() {
        let (everything, modules) = compute(&lock(), &BTreeSet::new());
        assert!(everything.is_none());
        assert!(modules.is_empty());
    }
}
