use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const FILE_NAME: &str = "lightning.lock";
pub const VERSION: u32 = 1;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Lock {
    pub version: u32,
    pub build_files_hash: String,
    /// Set when selection cannot be trusted (composite build): affected
    /// degrades to everything-affected with this reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<String>,
    pub modules: Vec<Module>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Module {
    /// Gradle project path, e.g. `:app` (`:` for the root project).
    pub path: String,
    /// Project directory relative to the Gradle root (`.` for the root).
    pub dir: String,
    /// Declared source-set dirs relative to the Gradle root; may leave the
    /// module dir (`srcDir("../shared")`).
    pub source_dirs: Vec<String>,
    /// Task names registered on the project.
    pub tasks: Vec<String>,
    /// Declared inter-module dependency edges.
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Dep {
    pub path: String,
    pub kind: EdgeKind,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Main,
    Test,
}

/// Why a module is in the affected set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    Changed,
    MainDependency,
    TestDependency,
    Everything,
}

impl Lock {
    pub fn load(dir: &Path) -> Result<Lock, String> {
        let path = dir.join(FILE_NAME);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let lock: Lock =
            serde_json::from_str(&text).map_err(|e| format!("invalid {}: {e}", path.display()))?;
        if lock.version != VERSION {
            return Err(format!(
                "{} has version {}, this build understands {VERSION} — run `lightning sync`",
                path.display(),
                lock.version
            ));
        }
        Ok(lock)
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let path = dir.join(FILE_NAME);
        let mut text = serde_json::to_string_pretty(self).expect("lock serializes");
        text.push('\n');
        std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

/// Affected closure over typed edges: a module is affected iff it changed,
/// a changed module is reachable from it via main edges transitively, or one
/// of its direct test edges points into that main-affected set. Test edges do
/// not propagate further.
pub fn closure(modules: &[Module], changed: &BTreeSet<String>) -> BTreeMap<String, Reason> {
    let mut rev_main: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for m in modules {
        for d in &m.deps {
            if d.kind == EdgeKind::Main && d.path != m.path {
                rev_main.entry(&d.path).or_default().push(&m.path);
            }
        }
    }
    let mut affected: BTreeMap<String, Reason> = BTreeMap::new();
    let mut queue: Vec<&str> = Vec::new();
    for m in modules {
        if changed.contains(&m.path) {
            affected.insert(m.path.clone(), Reason::Changed);
            queue.push(&m.path);
        }
    }
    while let Some(path) = queue.pop() {
        for &dependent in rev_main.get(path).into_iter().flatten() {
            if !affected.contains_key(dependent) {
                affected.insert(dependent.to_string(), Reason::MainDependency);
                queue.push(dependent);
            }
        }
    }
    // one non-propagating hop over test edges into the main-affected set
    let main_affected: BTreeSet<String> = affected.keys().cloned().collect();
    for m in modules {
        if affected.contains_key(&m.path) {
            continue;
        }
        if m.deps
            .iter()
            .any(|d| d.kind == EdgeKind::Test && main_affected.contains(&d.path))
        {
            affected.insert(m.path.clone(), Reason::TestDependency);
        }
    }
    affected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(path: &str, deps: &[(&str, EdgeKind)]) -> Module {
        Module {
            path: path.into(),
            dir: path.trim_start_matches(':').replace(':', "/"),
            source_dirs: vec![],
            tasks: vec![],
            deps: deps
                .iter()
                .map(|(p, k)| Dep {
                    path: (*p).into(),
                    kind: *k,
                })
                .collect(),
        }
    }

    fn changed(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| (*p).into()).collect()
    }

    #[test]
    fn main_edges_propagate_transitively() {
        let modules = vec![
            module(":core", &[]),
            module(":lib", &[(":core", EdgeKind::Main)]),
            module(":app", &[(":lib", EdgeKind::Main)]),
            module(":free", &[]),
        ];
        let got = closure(&modules, &changed(&[":core"]));
        assert_eq!(got.get(":core"), Some(&Reason::Changed));
        assert_eq!(got.get(":lib"), Some(&Reason::MainDependency));
        assert_eq!(got.get(":app"), Some(&Reason::MainDependency));
        assert!(!got.contains_key(":free"));
    }

    #[test]
    fn test_edges_reach_main_affected_but_do_not_propagate() {
        let modules = vec![
            module(":core", &[]),
            module(":fixtures", &[(":core", EdgeKind::Main)]),
            module(":lib", &[(":fixtures", EdgeKind::Test)]),
            module(":app", &[(":lib", EdgeKind::Main)]),
        ];
        // :core changed → :fixtures main-affected → :lib via its test edge
        // (one hop onto the main-affected set), but :app must not follow
        // :lib's test-only involvement.
        let got = closure(&modules, &changed(&[":core"]));
        assert_eq!(got.get(":fixtures"), Some(&Reason::MainDependency));
        assert_eq!(got.get(":lib"), Some(&Reason::TestDependency));
        assert!(!got.contains_key(":app"));
    }

    #[test]
    fn lock_roundtrips_and_rejects_other_versions() {
        let dir = std::env::temp_dir().join(format!("lightning-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = Lock {
            version: VERSION,
            build_files_hash: "abc".into(),
            unsupported: None,
            modules: vec![module(":app", &[(":lib", EdgeKind::Test)])],
        };
        lock.save(&dir).unwrap();
        let loaded = Lock::load(&dir).unwrap();
        assert_eq!(loaded.modules[0].deps[0].kind, EdgeKind::Test);

        let text = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        std::fs::write(
            dir.join(FILE_NAME),
            text.replace("\"version\": 1", "\"version\": 99"),
        )
        .unwrap();
        assert!(Lock::load(&dir).unwrap_err().contains("lightning sync"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Property test (FN-never): the optimized closure must equal an
    // independently coded naive reference on random DAGs and diffs, and be a
    // superset of the changed and main-affected sets.

    fn xorshift(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    /// Naive reference: per module, DFS "can I reach a changed module via
    /// main edges only", then a literal one-hop test-edge sweep.
    fn naive_closure(modules: &[Module], changed: &BTreeSet<String>) -> BTreeSet<String> {
        let by_path: BTreeMap<&str, &Module> =
            modules.iter().map(|m| (m.path.as_str(), m)).collect();
        fn reaches_changed(
            from: &str,
            by_path: &BTreeMap<&str, &Module>,
            changed: &BTreeSet<String>,
            visited: &mut BTreeSet<String>,
        ) -> bool {
            if changed.contains(from) {
                return true;
            }
            for d in by_path[from]
                .deps
                .iter()
                .filter(|d| d.kind == EdgeKind::Main)
            {
                if visited.insert(d.path.clone())
                    && reaches_changed(&d.path, by_path, changed, visited)
                {
                    return true;
                }
            }
            false
        }
        let mut main_affected: BTreeSet<String> = BTreeSet::new();
        for m in modules {
            if reaches_changed(&m.path, &by_path, changed, &mut BTreeSet::new()) {
                main_affected.insert(m.path.clone());
            }
        }
        let mut all = main_affected.clone();
        for m in modules {
            if m.deps
                .iter()
                .any(|d| d.kind == EdgeKind::Test && main_affected.contains(&d.path))
            {
                all.insert(m.path.clone());
            }
        }
        all
    }

    #[test]
    fn property_random_dags_match_naive_reference() {
        let mut state: u64 = 0x195eed;
        for _ in 0..500 {
            let n = (xorshift(&mut state) % 20 + 1) as usize;
            let mut modules: Vec<Module> = Vec::new();
            for i in 0..n {
                let mut deps = Vec::new();
                for j in 0..i {
                    // edges only towards lower indices → acyclic
                    match xorshift(&mut state) % 5 {
                        0 => deps.push((format!(":m{j}"), EdgeKind::Main)),
                        1 => deps.push((format!(":m{j}"), EdgeKind::Test)),
                        _ => {}
                    }
                }
                modules.push(Module {
                    path: format!(":m{i}"),
                    dir: format!("m{i}"),
                    source_dirs: vec![],
                    tasks: vec![],
                    deps: deps
                        .into_iter()
                        .map(|(path, kind)| Dep { path, kind })
                        .collect(),
                });
            }
            let changed: BTreeSet<String> = (0..n)
                .filter(|_| xorshift(&mut state).is_multiple_of(4))
                .map(|i| format!(":m{i}"))
                .collect();
            let got = closure(&modules, &changed);
            let got_set: BTreeSet<String> = got.keys().cloned().collect();
            let expected = naive_closure(&modules, &changed);
            assert_eq!(got_set, expected, "changed: {changed:?}");
            // superset invariants: affected ⊇ changed and ⊇ the closure over
            // main edges alone (test edges only ever widen the set)
            assert!(got_set.is_superset(&changed));
            let stripped: Vec<Module> = modules
                .iter()
                .map(|m| Module {
                    path: m.path.clone(),
                    dir: m.dir.clone(),
                    source_dirs: vec![],
                    tasks: vec![],
                    deps: m
                        .deps
                        .iter()
                        .filter(|d| d.kind == EdgeKind::Main)
                        .cloned()
                        .collect(),
                })
                .collect();
            let main_set: BTreeSet<String> = closure(&stripped, &changed).into_keys().collect();
            assert!(got_set.is_superset(&main_set));
        }
    }
}
