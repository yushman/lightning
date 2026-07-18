use std::path::Path;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub affected: Affected,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Affected {
    /// Base ref for the diff (default: origin/main).
    pub base: Option<String>,
    /// Opt-in globs excluded from the diff before mapping. No defaults.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Extra globs added to the lock invalidation hash set.
    #[serde(default)]
    pub invalidate_on: Vec<String>,
}

pub const FILE_NAME: &str = "lightning.toml";

pub fn load(dir: &Path) -> Result<Config, String> {
    let path = dir.join(FILE_NAME);
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("invalid {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_affected_keys_and_defaults() {
        let cfg: Config = toml::from_str(
            r#"
            [affected]
            base = "origin/develop"
            ignore = ["docs/**"]
            "#,
        )
        .unwrap();
        assert_eq!(cfg.affected.base.as_deref(), Some("origin/develop"));
        assert_eq!(cfg.affected.ignore, vec!["docs/**"]);
        assert!(cfg.affected.invalidate_on.is_empty());

        let empty: Config = toml::from_str("").unwrap();
        assert!(empty.affected.base.is_none());
    }

    #[test]
    fn rejects_unknown_keys() {
        let err = toml::from_str::<Config>("[affected]\nignored = [\"docs\"]").unwrap_err();
        assert!(err.to_string().contains("ignored"));
    }
}
