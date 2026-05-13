use std::process::Command;

use anyhow::{Context, Result};

use crate::config::SnippetConfig;

pub fn resolve_snippet(snippet: &SnippetConfig) -> Result<String> {
    if let Some(body) = &snippet.body {
        return Ok(body.clone());
    }

    if let Some(command) = &snippet.command {
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .with_context(|| format!("failed to run command for snippet '{}'", snippet.key))?;

        if !output.status.success() {
            anyhow::bail!(
                "command snippet '{}' failed: {}",
                snippet.key,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let raw = String::from_utf8_lossy(&output.stdout).into_owned();
        return Ok(trim_trailing_newlines(raw));
    }

    anyhow::bail!("invalid snippet '{}'", snippet.key)
}

fn trim_trailing_newlines(mut s: String) -> String {
    while s.ends_with('\n') || s.ends_with('\r') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(key: &str, name: &str, body: &str) -> SnippetConfig {
        SnippetConfig {
            key: key.into(),
            name: name.into(),
            body: Some(body.into()),
            command: None,
        }
    }

    fn cmd(key: &str, name: &str, command: &str) -> SnippetConfig {
        SnippetConfig {
            key: key.into(),
            name: name.into(),
            body: None,
            command: Some(command.into()),
        }
    }

    #[test]
    fn body_snippet_returns_verbatim() {
        let s = body("k", "n", "hello");
        assert_eq!(resolve_snippet(&s).unwrap(), "hello");
    }

    #[test]
    fn body_snippet_with_trailing_newline_is_preserved() {
        let s = body("k", "n", "hello\n");
        assert_eq!(resolve_snippet(&s).unwrap(), "hello\n");
    }

    #[test]
    fn invalid_snippet_neither_field_errors() {
        let s = SnippetConfig {
            key: "k".into(),
            name: "n".into(),
            body: None,
            command: None,
        };
        let err = resolve_snippet(&s).unwrap_err().to_string();
        assert!(err.contains("invalid snippet"), "{err}");
    }

    #[test]
    #[cfg(unix)]
    fn command_snippet_trims_trailing_newline() {
        let s = cmd("d", "Date-ish", "printf 'hello\\n'");
        assert_eq!(resolve_snippet(&s).unwrap(), "hello");
    }

    #[test]
    #[cfg(unix)]
    fn command_snippet_failure_propagates() {
        let s = cmd("x", "Bad", "exit 7");
        let err = resolve_snippet(&s).unwrap_err().to_string();
        assert!(err.contains("command snippet 'x' failed"), "{err}");
    }

    #[test]
    fn trim_trailing_newlines_handles_crlf() {
        assert_eq!(trim_trailing_newlines("a\r\n\r\n".into()), "a");
        assert_eq!(trim_trailing_newlines("a".into()), "a");
        assert_eq!(trim_trailing_newlines("a \n".into()), "a ");
    }
}
