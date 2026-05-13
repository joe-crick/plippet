use std::io::Write;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};

use crate::tools::{session_kind, SessionKind};

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    match session_kind() {
        SessionKind::Wayland => copy_via("wl-copy", &[], text, "wl-clipboard"),
        SessionKind::X11 => copy_via(
            "xclip",
            &["-selection", "clipboard"],
            text,
            "xclip (X11 session)",
        ),
    }
}

fn copy_via(bin: &str, args: &[&str], text: &str, install_hint: &str) -> Result<()> {
    let child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {bin}; install {install_hint}"))?;
    feed_stdin_and_wait(child, text, bin)
}

fn feed_stdin_and_wait(mut child: Child, text: &str, bin: &str) -> Result<()> {
    {
        let stdin = child
            .stdin
            .as_mut()
            .with_context(|| format!("failed to open {bin} stdin"))?;
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("{bin} exited with status {status}");
    }
    Ok(())
}
