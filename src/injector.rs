use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use directories::BaseDirs;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PasteBackend {
    /// Heuristic: portal on GNOME/KDE, wtype elsewhere.
    Auto,
    /// virtual-keyboard-unstable-v1 via the `wtype` binary. Wlroots only.
    Wtype,
    /// XDG Desktop Portal RemoteDesktop. Works on GNOME / KDE / wlroots.
    Portal,
}

pub fn paste(backend: PasteBackend) -> Result<()> {
    // Give the compositor and clipboard owner a short moment to observe the
    // new selection before we synthesize Ctrl+V. Avoids stale-clipboard
    // pastes seen on some systems.
    thread::sleep(Duration::from_millis(25));

    match resolve(backend) {
        PasteBackend::Wtype => paste_wtype(),
        PasteBackend::Portal => paste_portal(),
        PasteBackend::Auto => unreachable!("resolve() never returns Auto"),
    }
}

fn resolve(backend: PasteBackend) -> PasteBackend {
    match backend {
        PasteBackend::Auto => describe_auto().0,
        explicit => explicit,
    }
}

/// Returns the backend `Auto` would pick and a one-line reason.
pub fn describe_auto() -> (PasteBackend, String) {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let lower = desktop.to_lowercase();
    if lower.contains("gnome")
        || lower.contains("kde")
        || lower.contains("plasma")
        || lower.contains("kwin")
    {
        (
            PasteBackend::Portal,
            format!("XDG_CURRENT_DESKTOP={desktop}"),
        )
    } else if desktop.is_empty() {
        (PasteBackend::Wtype, "XDG_CURRENT_DESKTOP unset".to_string())
    } else {
        (
            PasteBackend::Wtype,
            format!("XDG_CURRENT_DESKTOP={desktop}"),
        )
    }
}

fn paste_wtype() -> Result<()> {
    let status = Command::new("wtype")
        .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
        .status()
        .context(
            "failed to spawn wtype; install it (wlroots-based compositors only) \
             or use --paste-backend portal on GNOME/KDE",
        )?;

    if !status.success() {
        anyhow::bail!("wtype exited with status {status}");
    }
    Ok(())
}

fn paste_portal() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for portal call")?;
    runtime.block_on(paste_portal_async())
}

async fn paste_portal_async() -> Result<()> {
    use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
    use ashpd::desktop::PersistMode;

    // X11 keysyms for Ctrl+V.
    const KEYSYM_CONTROL_L: i32 = 0xffe3;
    const KEYSYM_V: i32 = 0x0076;

    let proxy = RemoteDesktop::new()
        .await
        .context("failed to connect to XDG Desktop Portal; is xdg-desktop-portal running?")?;

    let session = proxy
        .create_session()
        .await
        .context("portal CreateSession failed")?;

    let token = load_token().ok();
    proxy
        .select_devices(
            &session,
            DeviceType::Keyboard.into(),
            token.as_deref(),
            PersistMode::ExplicitlyRevoked,
        )
        .await
        .context("portal SelectDevices failed")?;

    let request = proxy.start(&session, None).await.context(
        "portal Start failed; the user must approve the keyboard-control prompt the first time",
    )?;
    let selected = request
        .response()
        .context("portal returned no SelectedDevices")?;

    if let Some(new_token) = selected.restore_token() {
        if let Err(e) = save_token(new_token) {
            eprintln!("warning: failed to persist portal restore token: {e:#}");
        }
    }

    proxy
        .notify_keyboard_keysym(&session, KEYSYM_CONTROL_L, KeyState::Pressed)
        .await
        .context("portal Ctrl down failed")?;
    proxy
        .notify_keyboard_keysym(&session, KEYSYM_V, KeyState::Pressed)
        .await
        .context("portal V down failed")?;
    proxy
        .notify_keyboard_keysym(&session, KEYSYM_V, KeyState::Released)
        .await
        .context("portal V up failed")?;
    proxy
        .notify_keyboard_keysym(&session, KEYSYM_CONTROL_L, KeyState::Released)
        .await
        .context("portal Ctrl up failed")?;

    Ok(())
}

fn token_path() -> Result<PathBuf> {
    let dirs = BaseDirs::new()
        .ok_or_else(|| anyhow!("could not determine user config directory"))?;
    Ok(dirs.config_dir().join("plippet").join("portal_token"))
}

fn load_token() -> Result<String> {
    let path = token_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("no portal token at {}", path.display()))?;
    Ok(raw.trim().to_string())
}

fn save_token(token: &str) -> Result<()> {
    let path = token_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, token).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
