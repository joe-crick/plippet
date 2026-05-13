use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use directories::BaseDirs;

use crate::tools::{is_gnome_or_kde_desktop, session_kind_from_env, SessionKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PasteBackend {
    /// Heuristic: portal on GNOME/KDE, wtype on other Wayland setups,
    /// xdotool on X11.
    Auto,
    /// virtual-keyboard-unstable-v1 via the `wtype` binary. Wayland (wlroots) only.
    Wtype,
    /// XDG Desktop Portal RemoteDesktop. Works on GNOME / KDE / wlroots Wayland.
    Portal,
    /// X11 paste via the `xdotool` binary.
    Xdotool,
}

/// Which key combination to synthesize for "paste".
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PasteKeys {
    /// Ctrl+V — the universal paste shortcut for GUI text inputs (browsers,
    /// editors, address bars, chat apps, etc.). The default.
    CtrlV,
    /// Ctrl+Shift+V — the conventional paste shortcut in Linux terminals
    /// (GNOME Terminal, Foot, Kitty, Alacritty, …). Use this if your hotkey
    /// is meant to drop snippets into a terminal prompt.
    CtrlShiftV,
}

// X11 keysym constants for the keys we synthesize.
const KEYSYM_CONTROL_L: i32 = 0xffe3;
const KEYSYM_SHIFT_L: i32 = 0xffe1;
const KEYSYM_V: i32 = 0x0076;

impl PasteKeys {
    /// `(modifier_keysyms, main_keysym)` — modifiers are pressed in order
    /// before the main key and released in reverse order after.
    fn keysyms(self) -> (&'static [i32], i32) {
        match self {
            PasteKeys::CtrlV => (&[KEYSYM_CONTROL_L], KEYSYM_V),
            PasteKeys::CtrlShiftV => (&[KEYSYM_CONTROL_L, KEYSYM_SHIFT_L], KEYSYM_V),
        }
    }

    /// Argv for `wtype`.
    fn wtype_args(self) -> &'static [&'static str] {
        match self {
            PasteKeys::CtrlV => &["-M", "ctrl", "-k", "v", "-m", "ctrl"],
            PasteKeys::CtrlShiftV => &[
                "-M", "ctrl", "-M", "shift", "-k", "v", "-m", "shift", "-m", "ctrl",
            ],
        }
    }

    /// `xdotool key` argument.
    fn xdotool_key(self) -> &'static str {
        match self {
            PasteKeys::CtrlV => "ctrl+v",
            PasteKeys::CtrlShiftV => "ctrl+shift+v",
        }
    }
}

pub fn paste(backend: PasteBackend, keys: PasteKeys) -> Result<()> {
    // Give the compositor time both to observe the new clipboard selection
    // and to transfer keyboard focus from the picker window back to the
    // previously-focused app. 25 ms is fine on wlroots compositors but
    // turns out to be too tight on GNOME, where focus transfer after a
    // regular xdg-shell window closes can take noticeably longer. 150 ms
    // is still imperceptible and reliable across compositors observed so far.
    thread::sleep(Duration::from_millis(150));

    match resolve(backend) {
        PasteBackend::Wtype => paste_wtype(keys),
        PasteBackend::Portal => paste_portal(keys),
        PasteBackend::Xdotool => paste_xdotool(keys),
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
    auto_from_env(
        env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        env::var("XDG_SESSION_TYPE").ok().as_deref(),
        env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )
}

/// Pure auto-resolution. Tests pass env values explicitly so they don't have
/// to mutate the process environment.
fn auto_from_env(
    desktop: Option<&str>,
    session_type: Option<&str>,
    wayland_display: Option<&str>,
) -> (PasteBackend, String) {
    match session_kind_from_env(session_type, wayland_display) {
        SessionKind::X11 => {
            let reason = match session_type {
                Some(s) => format!("X11 session (XDG_SESSION_TYPE={s})"),
                None => "X11 session (no Wayland signals)".to_string(),
            };
            (PasteBackend::Xdotool, reason)
        }
        SessionKind::Wayland => {
            let raw = desktop.unwrap_or("");
            if raw.is_empty() {
                return (
                    PasteBackend::Wtype,
                    "Wayland session, XDG_CURRENT_DESKTOP unset".to_string(),
                );
            }
            if is_gnome_or_kde_desktop(Some(raw)) {
                (
                    PasteBackend::Portal,
                    format!("Wayland session, XDG_CURRENT_DESKTOP={raw}"),
                )
            } else {
                (
                    PasteBackend::Wtype,
                    format!("Wayland session, XDG_CURRENT_DESKTOP={raw}"),
                )
            }
        }
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

fn paste_xdotool() -> Result<()> {
    let status = Command::new("xdotool")
        .args(["key", "ctrl+v"])
        .status()
        .context("failed to spawn xdotool; install xdotool for X11 paste")?;
    if !status.success() {
        anyhow::bail!("xdotool exited with status {status}");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: assume a Wayland session and supply the desktop hint.
    fn wl(desktop: Option<&str>) -> (PasteBackend, String) {
        auto_from_env(desktop, Some("wayland"), Some("wayland-0"))
    }

    #[test]
    fn auto_wayland_unset_desktop_picks_wtype() {
        let (backend, reason) = wl(None);
        assert_eq!(backend, PasteBackend::Wtype);
        assert!(reason.contains("unset"), "{reason}");
    }

    #[test]
    fn auto_wayland_empty_desktop_picks_wtype() {
        let (backend, reason) = wl(Some(""));
        assert_eq!(backend, PasteBackend::Wtype);
        assert!(reason.contains("unset"), "{reason}");
    }

    #[test]
    fn auto_gnome_picks_portal() {
        assert_eq!(wl(Some("GNOME")).0, PasteBackend::Portal);
        // colon-separated chain (real GNOME session value)
        assert_eq!(wl(Some("GNOME-Classic:GNOME:")).0, PasteBackend::Portal);
    }

    #[test]
    fn auto_kde_picks_portal() {
        assert_eq!(wl(Some("KDE")).0, PasteBackend::Portal);
        assert_eq!(wl(Some("Plasma")).0, PasteBackend::Portal);
        assert_eq!(wl(Some("KWin")).0, PasteBackend::Portal);
    }

    #[test]
    fn auto_is_case_insensitive() {
        assert_eq!(wl(Some("gnome")).0, PasteBackend::Portal);
        assert_eq!(wl(Some("kde")).0, PasteBackend::Portal);
        assert_eq!(wl(Some("PLASMA")).0, PasteBackend::Portal);
    }

    #[test]
    fn auto_sway_picks_wtype() {
        assert_eq!(wl(Some("sway")).0, PasteBackend::Wtype);
        assert_eq!(wl(Some("Hyprland")).0, PasteBackend::Wtype);
        assert_eq!(wl(Some("river")).0, PasteBackend::Wtype);
    }

    #[test]
    fn auto_reason_echoes_raw_desktop_value() {
        let (_, reason) = wl(Some("GNOME-Classic:GNOME:"));
        assert!(reason.contains("GNOME-Classic:GNOME:"), "{reason}");
    }

    #[test]
    fn auto_x11_picks_xdotool_regardless_of_desktop() {
        let (backend, reason) = auto_from_env(Some("XFCE"), Some("x11"), None);
        assert_eq!(backend, PasteBackend::Xdotool);
        assert!(reason.contains("X11"), "{reason}");
        // even for desktops that would otherwise resolve to portal, X11 wins
        assert_eq!(
            auto_from_env(Some("GNOME"), Some("x11"), None).0,
            PasteBackend::Xdotool
        );
    }

    #[test]
    fn auto_no_env_at_all_falls_back_to_x11() {
        let (backend, _) = auto_from_env(None, None, None);
        assert_eq!(backend, PasteBackend::Xdotool);
    }

    #[test]
    fn auto_wayland_display_alone_is_wayland() {
        // session_type unset but WAYLAND_DISPLAY set → treat as Wayland.
        assert_eq!(
            auto_from_env(Some("GNOME"), None, Some("wayland-0")).0,
            PasteBackend::Portal
        );
    }
}
