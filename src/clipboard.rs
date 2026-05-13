use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn wl-copy; install wl-clipboard")?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .context("failed to open wl-copy stdin")?;
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;

    if !status.success() {
        anyhow::bail!("wl-copy exited with status {status}");
    }

    Ok(())
}
