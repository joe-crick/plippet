use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub snippet: Vec<SnippetConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SnippetConfig {
    pub key: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl Config {
    pub fn default_path() -> Result<PathBuf> {
        let dirs = BaseDirs::new()
            .ok_or_else(|| anyhow!("could not determine user config directory"))?;
        Ok(dirs.config_dir().join("plippet").join("snippets.toml"))
    }

    pub fn load_default() -> Result<Self> {
        let path = Self::default_path()?;
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                anyhow::bail!(
                    "config file not found:\n  {}\n\nCreate it with:\n  mkdir -p ~/.config/plippet\n  cp examples/snippets.toml ~/.config/plippet/snippets.toml",
                    path.display()
                );
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("failed to read config at {}", path.display())
                });
            }
        };

        let config: Config = toml::from_str(&raw)
            .with_context(|| format!("failed to parse TOML config at {}", path.display()))?;

        Ok(config)
    }

    /// Atomic-ish save: writes to `<path>.tmp` then renames into place. Creates
    /// the parent directory if it doesn't exist yet.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let content =
            toml::to_string_pretty(self).context("failed to serialize config to TOML")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, content.as_bytes())
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| {
            format!(
                "failed to rename {} to {}",
                tmp.display(),
                path.display()
            )
        })?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        let mut keys = HashSet::new();

        for snippet in &self.snippet {
            if snippet.key.trim().is_empty() {
                anyhow::bail!("snippet key cannot be empty");
            }

            if snippet.name.trim().is_empty() {
                anyhow::bail!("snippet '{}' has an empty name", snippet.key);
            }

            if !keys.insert(snippet.key.clone()) {
                anyhow::bail!("duplicate snippet key: {}", snippet.key);
            }

            match (&snippet.body, &snippet.command) {
                (Some(_), Some(_)) => {
                    anyhow::bail!(
                        "snippet '{}' must not define both body and command",
                        snippet.key
                    );
                }
                (None, None) => {
                    anyhow::bail!(
                        "snippet '{}' must define either body or command",
                        snippet.key
                    );
                }
                _ => {}
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_src: &str) -> Result<Config> {
        let cfg: Config = toml::from_str(toml_src)?;
        Ok(cfg)
    }

    #[test]
    fn validates_body_snippet() {
        let cfg = parse(
            r#"
            [[snippet]]
            key = "sig"
            name = "Signoff"
            body = "Best,\nJ"
            "#,
        )
        .unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn validates_command_snippet() {
        let cfg = parse(
            r#"
            [[snippet]]
            key = "today"
            name = "Today"
            command = "date +%Y-%m-%d"
            "#,
        )
        .unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_empty_key() {
        let cfg = parse(
            r#"
            [[snippet]]
            key = ""
            name = "x"
            body = "y"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("key cannot be empty"), "{err}");
    }

    #[test]
    fn rejects_empty_name() {
        let cfg = parse(
            r#"
            [[snippet]]
            key = "k"
            name = ""
            body = "y"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("empty name"), "{err}");
    }

    #[test]
    fn rejects_duplicate_keys() {
        let cfg = parse(
            r#"
            [[snippet]]
            key = "k"
            name = "a"
            body = "x"

            [[snippet]]
            key = "k"
            name = "b"
            body = "y"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate snippet key"), "{err}");
    }

    #[test]
    fn rejects_both_body_and_command() {
        let cfg = parse(
            r#"
            [[snippet]]
            key = "k"
            name = "a"
            body = "x"
            command = "echo hi"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("must not define both"), "{err}");
    }

    #[test]
    fn rejects_neither_body_nor_command() {
        let cfg = parse(
            r#"
            [[snippet]]
            key = "k"
            name = "a"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("must define either"), "{err}");
    }

    #[test]
    fn default_path_ends_with_plippet_snippets_toml() {
        let path = Config::default_path().unwrap();
        let suffix = std::path::Path::new("plippet").join("snippets.toml");
        assert!(
            path.ends_with(&suffix),
            "{} should end with {}",
            path.display(),
            suffix.display()
        );
    }

    #[test]
    fn load_from_reads_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        std::fs::write(
            &path,
            r#"
            [[snippet]]
            key = "sig"
            name = "Signoff"
            body = "Best,\nJ"
            "#,
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.snippet.len(), 1);
        assert_eq!(cfg.snippet[0].key, "sig");
    }

    #[test]
    fn load_from_missing_file_returns_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        let err = Config::load_from(&path).unwrap_err().to_string();
        assert!(err.contains("config file not found"), "{err}");
        assert!(err.contains(&path.display().to_string()), "{err}");
        assert!(err.contains("mkdir -p ~/.config/plippet"), "{err}");
    }

    #[test]
    fn load_from_malformed_toml_errors_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is not valid TOML = [[[").unwrap();
        let err = Config::load_from(&path).unwrap_err().to_string();
        assert!(err.contains("failed to parse TOML"), "{err}");
        assert!(err.contains(&path.display().to_string()), "{err}");
    }

    #[test]
    fn empty_config_file_is_valid_and_has_no_snippets() {
        // empty TOML (no [[snippet]] tables) should parse — `snippet` is `#[serde(default)]`
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.toml");
        std::fs::write(&path, "").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.snippet.is_empty());
    }

    #[test]
    fn save_to_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        let cfg = Config {
            snippet: vec![
                SnippetConfig {
                    key: "sig".into(),
                    name: "Signoff".into(),
                    body: Some("Best,\nJ\n".into()),
                    command: None,
                },
                SnippetConfig {
                    key: "today".into(),
                    name: "Date".into(),
                    body: None,
                    command: Some("date +%Y-%m-%d".into()),
                },
            ],
        };
        cfg.save_to(&path).unwrap();

        let loaded = Config::load_from(&path).unwrap();
        loaded.validate().unwrap();
        assert_eq!(loaded.snippet.len(), 2);
        assert_eq!(loaded.snippet[0].key, "sig");
        assert_eq!(loaded.snippet[0].body.as_deref(), Some("Best,\nJ\n"));
        assert_eq!(loaded.snippet[0].command, None);
        assert_eq!(loaded.snippet[1].command.as_deref(), Some("date +%Y-%m-%d"));
        assert_eq!(loaded.snippet[1].body, None);
    }

    #[test]
    fn save_to_creates_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("snippets.toml");
        let cfg = Config { snippet: vec![] };
        cfg.save_to(&path).unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn save_to_omits_none_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        let cfg = Config {
            snippet: vec![SnippetConfig {
                key: "k".into(),
                name: "n".into(),
                body: Some("x".into()),
                command: None,
            }],
        };
        cfg.save_to(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("body"), "{content}");
        assert!(!content.contains("command"), "{content}");
    }

    #[test]
    fn save_to_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        std::fs::write(&path, "garbage that should be replaced").unwrap();
        let cfg = Config {
            snippet: vec![SnippetConfig {
                key: "k".into(),
                name: "n".into(),
                body: Some("x".into()),
                command: None,
            }],
        };
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.snippet.len(), 1);
        assert_eq!(loaded.snippet[0].key, "k");
    }
}
