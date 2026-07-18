//! Path helpers: all affected-selection matching happens on normalized,
//! forward-slash paths relative to the Gradle root (where lightning runs).

/// Normalize a forward-slash relative path: fold `.` and `x/..` pairs,
/// keep leading `..` components.
pub fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if matches!(out.last(), Some(&last) if last != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            p => out.push(p),
        }
    }
    if out.is_empty() {
        ".".into()
    } else {
        out.join("/")
    }
}

/// Rebase `path` (relative to a root) onto a subdirectory of that root given
/// by `prefix` (also root-relative, `.` for the root itself). The result may
/// start with `..` when the path lies outside the prefix.
pub fn rebase(path: &str, prefix: &str) -> String {
    let path = normalize(path);
    if prefix == "." || prefix.is_empty() {
        return path;
    }
    let mut path_parts: Vec<&str> = path.split('/').collect();
    let mut prefix_parts: Vec<&str> = prefix.split('/').collect();
    while !path_parts.is_empty() && !prefix_parts.is_empty() && path_parts[0] == prefix_parts[0] {
        path_parts.remove(0);
        prefix_parts.remove(0);
    }
    let mut out: Vec<&str> = prefix_parts.iter().map(|_| "..").collect();
    out.extend(path_parts);
    normalize(&out.join("/"))
}

/// True when `path` is `dir` itself or lies under it (both normalized).
pub fn is_under(path: &str, dir: &str) -> bool {
    if dir == "." {
        return !path.starts_with("../");
    }
    path == dir || path.starts_with(&format!("{dir}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_dots() {
        assert_eq!(normalize("core/../shared/src"), "shared/src");
        assert_eq!(normalize("./a/b/"), "a/b");
        assert_eq!(normalize("a/.."), ".");
        assert_eq!(normalize("../shared"), "../shared");
        assert_eq!(normalize("a/../../b"), "../b");
    }

    #[test]
    fn rebase_handles_outside_paths() {
        assert_eq!(rebase("android/app/A.java", "android"), "app/A.java");
        assert_eq!(rebase("README.md", "android"), "../README.md");
        assert_eq!(rebase("app/A.java", "."), "app/A.java");
    }

    #[test]
    fn is_under_matches_prefix_components() {
        assert!(is_under("app/src/A.java", "app"));
        assert!(!is_under("app2/src/A.java", "app"));
        assert!(is_under("anything", "."));
        assert!(!is_under("../outside", "."));
    }
}
