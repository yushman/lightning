use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::lock::{Dep, EdgeKind, Lock, Module, VERSION};
use crate::paths;

pub const INIT_SCRIPT: &str = include_str!("../assets/lightning.sync.init.gradle");

/// Fixed invalidation glob set: every file that can change the module graph.
const INVALIDATION_GLOBS: &[&str] = &[
    "**/*.gradle",
    "**/*.gradle.kts",
    "buildSrc/**",
    "build-logic/**",
    "gradle/libs.versions.toml",
    "gradle/wrapper/gradle-wrapper.properties",
    "gradle.properties",
    "local.properties",
    // lightning's own config shapes selection (ignore/invalidate_on), so a
    // change to it must force a re-sync instead of mapping into the graph
    "lightning.toml",
];

/// Directories skipped by the hash walk: generated files must not flap it.
const SKIPPED_DIRS: &[&str] = &[".git", ".gradle", "build"];

/// Test-typed configurations, a deliberately closed list (design D3):
/// anything else — testFixtures, androidTest, custom suites — stays `main`
/// so unknown configurations can only over-select, never under-select.
const TEST_CONFIGURATIONS: &[&str] = &[
    "testImplementation",
    "testApi",
    "testCompileOnly",
    "testRuntimeOnly",
];

pub fn edge_kind(configuration: &str) -> EdgeKind {
    if TEST_CONFIGURATIONS.contains(&configuration) {
        EdgeKind::Test
    } else {
        EdgeKind::Main
    }
}

pub fn invalidation_matchers(extra: &[String]) -> Result<Vec<glob::Pattern>, String> {
    INVALIDATION_GLOBS
        .iter()
        .copied()
        .map(str::to_string)
        .chain(extra.iter().cloned())
        .map(|g| glob::Pattern::new(&g).map_err(|e| format!("invalid glob {g:?}: {e}")))
        .collect()
}

/// `<dir>/**` invalidation globs for included-build roots recorded in the
/// lock (dir literals escaped so metacharacters cannot widen the pattern).
pub fn included_build_globs(dirs: &[String]) -> Vec<String> {
    dirs.iter()
        .map(|d| format!("{}/**", glob::Pattern::escape(d)))
        .collect()
}

pub fn matches_any(patterns: &[glob::Pattern], path: &str) -> bool {
    patterns.iter().any(|p| {
        p.matches(path)
            || p.as_str()
                .strip_prefix("**/")
                .is_some_and(|bare| glob::Pattern::new(bare).is_ok_and(|b| b.matches(path)))
    })
}

/// blake3 over the sorted invalidation file list: `path NUL len NUL bytes`.
pub fn build_files_hash(root: &Path, extra_globs: &[String]) -> Result<String, String> {
    let patterns = invalidation_matchers(extra_globs)?;
    let mut files: BTreeSet<String> = BTreeSet::new();
    collect_files(root, "", &patterns, &mut files)?;
    let mut hasher = blake3::Hasher::new();
    for rel in &files {
        let bytes = std::fs::read(root.join(rel)).map_err(|e| format!("cannot read {rel}: {e}"))?;
        hasher.update(rel.as_bytes());
        hasher.update(b"\x00");
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(b"\x00");
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn collect_files(
    root: &Path,
    prefix: &str,
    patterns: &[glob::Pattern],
    out: &mut BTreeSet<String>,
) -> Result<(), String> {
    let dir = if prefix.is_empty() {
        root.to_path_buf()
    } else {
        root.join(prefix)
    };
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {rel}: {e}"))?;
        if file_type.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                collect_files(root, &rel, patterns, out)?;
            }
        } else if file_type.is_file() && matches_any(patterns, &rel) {
            out.insert(rel);
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct Dump {
    unsupported: Option<String>,
    #[serde(default)]
    included_builds: Vec<DumpIncludedBuild>,
    modules: Vec<DumpModule>,
}

/// The script also dumps the included build's `name`; only the dir matters
/// here (names appear in unsupported reasons composed by the script).
#[derive(serde::Deserialize)]
struct DumpIncludedBuild {
    dir: String,
}

#[derive(serde::Deserialize)]
struct DumpModule {
    path: String,
    dir: String,
    source_dirs: Vec<String>,
    tasks: Vec<String>,
    deps: Vec<DumpDep>,
}

#[derive(serde::Deserialize)]
struct DumpDep {
    path: String,
    configuration: String,
}

/// Normalize the raw init-script dump into a deterministic lock: sorted
/// modules, sorted+deduped source dirs/tasks/edges, typed edges.
fn normalize(dump: Dump, build_files_hash: String) -> Lock {
    let mut modules: Vec<Module> = dump
        .modules
        .into_iter()
        .map(|m| {
            let source_dirs: BTreeSet<String> =
                m.source_dirs.iter().map(|d| paths::normalize(d)).collect();
            let tasks: BTreeSet<String> = m.tasks.into_iter().collect();
            let deps: BTreeSet<Dep> = m
                .deps
                .into_iter()
                .filter(|d| d.path != m.path)
                .map(|d| Dep {
                    path: d.path,
                    kind: edge_kind(&d.configuration),
                })
                .collect();
            Module {
                path: m.path,
                dir: paths::normalize(&m.dir),
                source_dirs: source_dirs.into_iter().collect(),
                tasks: tasks.into_iter().collect(),
                deps: deps.into_iter().collect(),
            }
        })
        .collect();
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    let included_builds: BTreeSet<String> = dump
        .included_builds
        .iter()
        .map(|b| paths::normalize(&b.dir))
        .collect();
    Lock {
        version: VERSION,
        build_files_hash,
        unsupported: dump.unsupported,
        included_builds: included_builds.into_iter().collect(),
        modules,
    }
}

fn is_gradle_root(dir: &Path) -> bool {
    [
        "settings.gradle",
        "settings.gradle.kts",
        "build.gradle",
        "build.gradle.kts",
    ]
    .iter()
    .any(|f| dir.join(f).is_file())
}

pub fn gradle_command(dir: &Path) -> Command {
    if dir.join("gradlew").is_file() {
        Command::new(dir.join("gradlew"))
    } else {
        Command::new("gradle")
    }
}

/// Run Gradle once with the sync init script and write `lightning.lock`.
pub fn run(dir: &Path, extra_invalidation_globs: &[String]) -> Result<Lock, String> {
    if !is_gradle_root(dir) {
        return Err(format!(
            "{} is not a Gradle root (no settings.gradle(.kts) or build.gradle(.kts)) — \
             run lightning sync from the Gradle root",
            dir.display()
        ));
    }
    let tmp = std::env::temp_dir();
    let pid = std::process::id();
    let script = tmp.join(format!("lightning-sync-{pid}.init.gradle"));
    let dump_path = tmp.join(format!("lightning-sync-{pid}.json"));
    std::fs::write(&script, INIT_SCRIPT)
        .map_err(|e| format!("cannot write {}: {e}", script.display()))?;
    let _ = std::fs::remove_file(&dump_path);

    eprintln!("lightning: syncing module graph via gradle...");
    let output = gradle_command(dir)
        .current_dir(dir)
        .arg("--init-script")
        .arg(&script)
        .arg(format!("-Plightning.lock.dump={}", dump_path.display()))
        .arg("-q")
        .arg("help")
        .output()
        .map_err(|e| format!("cannot run gradle: {e}"))?;
    let _ = std::fs::remove_file(&script);
    if !output.status.success() {
        let _ = std::fs::remove_file(&dump_path);
        return Err(format!(
            "gradle sync build failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let raw = std::fs::read_to_string(&dump_path)
        .map_err(|e| format!("gradle succeeded but produced no model dump: {e}"))?;
    let _ = std::fs::remove_file(&dump_path);
    let dump: Dump = serde_json::from_str(&raw).map_err(|e| format!("invalid model dump: {e}"))?;

    // the hash must watch included-build roots too: normalize first (dirs
    // come from the dump), then hash with the same glob set `affected` will
    // recompute from the lock
    let mut lock = normalize(dump, String::new());
    let mut globs = extra_invalidation_globs.to_vec();
    globs.extend(included_build_globs(&lock.included_builds));
    lock.build_files_hash = build_files_hash(dir, &globs)?;
    lock.save(dir)?;
    if let Some(reason) = &lock.unsupported {
        eprintln!("lightning: warning: {reason}; affected will select everything");
    } else if !lock.included_builds.is_empty() {
        eprintln!(
            "lightning: included builds without dependency substitution (plugin-only): {}",
            lock.included_builds.join(", ")
        );
    }
    eprintln!(
        "lightning: wrote {} ({} modules)",
        crate::lock::FILE_NAME,
        lock.modules.len()
    );
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_kind_uses_closed_test_list() {
        assert_eq!(edge_kind("testImplementation"), EdgeKind::Test);
        assert_eq!(edge_kind("testRuntimeOnly"), EdgeKind::Test);
        assert_eq!(edge_kind("implementation"), EdgeKind::Main);
        assert_eq!(edge_kind("api"), EdgeKind::Main);
        // FN-safety: fixtures/android/custom configurations stay main
        assert_eq!(edge_kind("testFixturesImplementation"), EdgeKind::Main);
        assert_eq!(edge_kind("androidTestImplementation"), EdgeKind::Main);
        assert_eq!(edge_kind("integrationTestImplementation"), EdgeKind::Main);
    }

    #[test]
    fn included_builds_normalize_sorted_and_deduped() {
        let dump = Dump {
            unsupported: None,
            included_builds: vec![
                DumpIncludedBuild {
                    dir: "gradle/plugins".into(),
                },
                DumpIncludedBuild {
                    dir: "./build-logic".into(),
                },
                DumpIncludedBuild {
                    dir: "gradle/plugins".into(),
                },
            ],
            modules: vec![],
        };
        let lock = normalize(dump, "h".into());
        assert_eq!(lock.included_builds, vec!["build-logic", "gradle/plugins"]);
    }

    #[test]
    fn included_build_globs_invalidate_their_roots_only() {
        let globs = included_build_globs(&["gradle/plugins".into()]);
        assert_eq!(globs, vec!["gradle/plugins/**"]);
        let patterns = invalidation_matchers(&globs).unwrap();
        assert!(matches_any(
            &patterns,
            "gradle/plugins/src/main/kotlin/Conventions.kt"
        ));
        assert!(matches_any(&patterns, "gradle/plugins/settings.gradle.kts"));
        assert!(!matches_any(&patterns, "gradle/pluginsX/src/A.kt"));
        assert!(!matches_any(&patterns, "app/src/main/java/A.java"));
        // metacharacters in the dir stay literal
        assert_eq!(
            included_build_globs(&["build[x]".into()]),
            vec!["build[[]x[]]/**"]
        );
    }

    #[test]
    fn invalidation_globs_match_expected_files() {
        let patterns = invalidation_matchers(&["ci/versions.txt".into()]).unwrap();
        for path in [
            "build.gradle",
            "settings.gradle.kts",
            "app/build.gradle",
            "deep/nested/module/build.gradle.kts",
            "buildSrc/src/main/kotlin/Conventions.kt",
            "build-logic/settings.gradle",
            "gradle/libs.versions.toml",
            "gradle/wrapper/gradle-wrapper.properties",
            "gradle.properties",
            "local.properties",
            "ci/versions.txt",
        ] {
            assert!(matches_any(&patterns, path), "{path} should match");
        }
        for path in [
            "app/src/main/java/A.java",
            "docs/readme.md",
            "app/gradle.properties",
        ] {
            assert!(!matches_any(&patterns, path), "{path} should not match");
        }
    }

    #[test]
    fn hash_is_stable_and_sensitive_to_build_files() {
        let dir = std::env::temp_dir().join(format!("lightning-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        std::fs::create_dir_all(dir.join("build")).unwrap();
        std::fs::write(dir.join("settings.gradle"), "include ':app'").unwrap();
        std::fs::write(dir.join("app/build.gradle"), "plugins {}").unwrap();
        std::fs::write(dir.join("app/A.java"), "class A {}").unwrap();

        let h1 = build_files_hash(&dir, &[]).unwrap();
        assert_eq!(h1, build_files_hash(&dir, &[]).unwrap());
        // sources do not invalidate
        std::fs::write(dir.join("app/A.java"), "class A { int x; }").unwrap();
        assert_eq!(h1, build_files_hash(&dir, &[]).unwrap());
        // generated files under build/ do not invalidate
        std::fs::write(dir.join("build/generated.gradle"), "x").unwrap();
        assert_eq!(h1, build_files_hash(&dir, &[]).unwrap());
        // build files do
        std::fs::write(dir.join("app/build.gradle"), "plugins { id 'java' }").unwrap();
        assert_ne!(h1, build_files_hash(&dir, &[]).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fixture_walk_collects_build_files_only() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/multimodule");
        let patterns = invalidation_matchers(&[]).unwrap();
        let mut files = BTreeSet::new();
        collect_files(&root, "", &patterns, &mut files).unwrap();
        for expected in [
            "settings.gradle",
            "core/build.gradle",
            "custom/build.gradle",
        ] {
            assert!(files.contains(expected), "missing {expected}");
        }
        assert!(
            !files
                .iter()
                .any(|f| f.ends_with(".java") || f.ends_with(".md"))
        );
    }

    #[test]
    fn normalize_sorts_dedups_and_types_edges() {
        let dump = Dump {
            unsupported: None,
            included_builds: vec![],
            modules: vec![
                DumpModule {
                    path: ":lib".into(),
                    dir: "lib".into(),
                    source_dirs: vec!["lib/src/main/java".into(), "lib/../shared/src".into()],
                    tasks: vec!["test".into(), "build".into(), "test".into()],
                    deps: vec![
                        DumpDep {
                            path: ":core".into(),
                            configuration: "api".into(),
                        },
                        DumpDep {
                            path: ":core".into(),
                            configuration: "compileClasspath".into(),
                        },
                        DumpDep {
                            path: ":fixtures".into(),
                            configuration: "testImplementation".into(),
                        },
                        DumpDep {
                            path: ":lib".into(),
                            configuration: "api".into(),
                        },
                    ],
                },
                DumpModule {
                    path: ":core".into(),
                    dir: "core".into(),
                    source_dirs: vec![],
                    tasks: vec![],
                    deps: vec![],
                },
            ],
        };
        let lock = normalize(dump, "h".into());
        assert!(lock.included_builds.is_empty());
        assert_eq!(lock.modules[0].path, ":core");
        let lib = &lock.modules[1];
        assert_eq!(lib.source_dirs, vec!["lib/src/main/java", "shared/src"]);
        assert_eq!(lib.tasks, vec!["build", "test"]);
        assert_eq!(
            lib.deps,
            vec![
                Dep {
                    path: ":core".into(),
                    kind: EdgeKind::Main
                },
                Dep {
                    path: ":fixtures".into(),
                    kind: EdgeKind::Test
                },
            ]
        );
    }
}
