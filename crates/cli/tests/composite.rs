//! Real-Gradle integration tests for composite-build sync. They need a
//! `gradle` on PATH (the fixtures have no wrapper) and skip loudly otherwise,
//! so `cargo test --workspace` stays green on machines without a JVM.

use std::path::{Path, PathBuf};
use std::process::Command;

fn gradle_available() -> bool {
    Command::new("gradle")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
}

fn setup(fixture: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(fixture);
    let dir = std::env::temp_dir().join(format!("lightning-{fixture}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&src, &dir);
    dir
}

fn lightning(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_lightning"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn plugin_only_composite_syncs_to_a_normal_lock() {
    if !gradle_available() {
        eprintln!("skipping: no gradle on PATH");
        return;
    }
    let dir = setup("composite-plugin-only");

    let (code, _, err) = lightning(&dir, &["sync"]);
    assert_eq!(code, 0, "sync failed:\n{err}");
    assert!(err.contains("plugin-only"), "{err}");
    let lock: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("lightning.lock")).unwrap())
            .unwrap();
    assert_eq!(lock["version"], 2);
    assert!(lock.get("unsupported").is_none(), "{lock}");
    assert_eq!(
        lock["included_builds"],
        serde_json::json!(["gradle/conventions"])
    );

    // selection works normally: a core change affects :app and :core only
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "."]);
    git(
        &dir,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    );
    std::fs::write(
        dir.join("core/src/main/java/core/Core.java"),
        "package core;\n\npublic class Core {\n    // changed\n}\n",
    )
    .unwrap();
    let (code, out, err) = lightning(&dir, &["affected", "--base-sha", "HEAD"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out.trim().lines().collect::<Vec<_>>(),
        vec![":app", ":core"]
    );
    assert!(!err.contains("selecting everything"), "{err}");

    // dynamic invalidation: a change under the included build's root (not
    // named build-logic) makes the lock stale
    std::fs::write(
        dir.join("gradle/conventions/src/main/groovy/extra.txt"),
        "not a build file by extension, still build logic\n",
    )
    .unwrap();
    let (code, _, err) = lightning(&dir, &["affected", "--base-sha", "HEAD"]);
    assert_eq!(code, 4, "expected stale lock:\n{err}");
    assert!(err.contains("lightning sync"), "{err}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn substituting_composite_keeps_the_refusal() {
    if !gradle_available() {
        eprintln!("skipping: no gradle on PATH");
        return;
    }
    let dir = setup("composite-substituting");

    let (code, _, err) = lightning(&dir, &["sync"]);
    assert_eq!(code, 0, "sync failed:\n{err}");
    assert!(err.contains("dependency substitution"), "{err}");
    let lock: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("lightning.lock")).unwrap())
            .unwrap();
    assert_eq!(lock["version"], 2);
    let reason = lock["unsupported"].as_str().expect("unsupported set");
    assert!(reason.contains("dependency substitution"), "{reason}");
    assert!(reason.contains("included-lib"), "{reason}");

    let _ = std::fs::remove_dir_all(&dir);
}
