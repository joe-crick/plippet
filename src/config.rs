use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use directories::BaseDirs;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub snippet: Vec<SnippetConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SnippetConfig {
    pub key: String,
    pub name: String,
    pub body: Option<String>,
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
}
