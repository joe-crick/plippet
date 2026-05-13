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

/// How to deliver the snippet to the focused window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PasteMode {
    /// Synthesize a paste shortcut (Ctrl+V by default; see `--paste-keys`).
    /// Fast, but relies on the focused app having that paste binding AND on
    /// the compositor not intercepting the chord. Empirically Ctrl+Shift+V
    /// is dropped by mutter on GNOME.
    Chord,
    /// Type each character of the snippet via key synthesis. Slower for
    /// long snippets but works in any focused text input regardless of
    /// paste bindings — terminals, password fields, search boxes, etc.
    /// ASCII printable characters + newline + tab only in v1; non-ASCII
    /// (emoji, accented letters) is rejected with an error.
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteTarget {
    x11_window_id: String,
}

// X11 keysym constants for the keys we synthesize via the portal. We use
// keysyms (not evdev keycodes) on GNOME: empirically mutter's portal
// implementation of `notify_keyboard_keycode` doesn't route the events
// reliably, while `notify_keyboard_keysym` does.
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

pub fn paste(
    backend: PasteBackend,
    keys: PasteKeys,
    mode: PasteMode,
    target: Option<&PasteTarget>,
    text: &str,
) -> Result<()> {
    let resolved = resolve(backend);
    prepare_focus(resolved, target)?;
    match mode {
        PasteMode::Chord => match resolved {
            PasteBackend::Wtype => paste_chord_wtype(keys),
            PasteBackend::Portal => paste_chord_portal(keys),
            PasteBackend::Xdotool => paste_chord_xdotool(keys),
            PasteBackend::Auto => unreachable!("resolve() never returns Auto"),
        },
        PasteMode::Type => {
            validate_typeable(text)?;
            match resolved {
                PasteBackend::Wtype => paste_text_wtype(text),
                PasteBackend::Portal => paste_text_portal(text),
                PasteBackend::Xdotool => paste_text_xdotool(text),
                PasteBackend::Auto => unreachable!("resolve() never returns Auto"),
            }
        }
    }
}

pub fn capture_target(backend: PasteBackend) -> Result<Option<PasteTarget>> {
    match resolve(backend) {
        PasteBackend::Xdotool => Ok(Some(capture_x11_target()?)),
        PasteBackend::Wtype | PasteBackend::Portal => Ok(None),
        PasteBackend::Auto => unreachable!("resolve() never returns Auto"),
    }
}

fn resolve(backend: PasteBackend) -> PasteBackend {
    match backend {
        PasteBackend::Auto => describe_auto().0,
        explicit => explicit,
    }
}

fn prepare_focus(resolved: PasteBackend, target: Option<&PasteTarget>) -> Result<()> {
    match resolved {
        PasteBackend::Xdotool => {
            if let Some(target) = target {
                target.activate()?;
            }
            thread::sleep(Duration::from_millis(50));
            Ok(())
        }
        PasteBackend::Wtype | PasteBackend::Portal => {
            // Give the compositor time both to observe the new clipboard
            // selection and — more importantly on GNOME — to transfer
            // keyboard focus from the picker window back to the
            // previously-focused app.
            thread::sleep(Duration::from_millis(500));
            Ok(())
        }
        PasteBackend::Auto => unreachable!("resolve() never returns Auto"),
    }
}

fn capture_x11_target() -> Result<PasteTarget> {
    let output = Command::new("xdotool")
        .arg("getactivewindow")
        .output()
        .context("failed to spawn xdotool; install xdotool for X11 paste")?;
    if !output.status.success() {
        anyhow::bail!(
            "xdotool getactivewindow exited with status {}",
            output.status
        );
    }

    let x11_window_id = parse_xdotool_window_id(&output.stdout)
        .ok_or_else(|| anyhow!("xdotool getactivewindow returned no window id"))?;
    Ok(PasteTarget { x11_window_id })
}

fn parse_xdotool_window_id(stdout: &[u8]) -> Option<String> {
    let raw = String::from_utf8_lossy(stdout);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

impl PasteTarget {
    fn activate(&self) -> Result<()> {
        let status = Command::new("xdotool")
            .args(["windowactivate", "--sync", &self.x11_window_id])
            .status()
            .context("failed to spawn xdotool for X11 focus restore")?;
        if !status.success() {
            anyhow::bail!("xdotool windowactivate exited with status {status}");
        }
        Ok(())
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

fn paste_chord_wtype(keys: PasteKeys) -> Result<()> {
    let status = Command::new("wtype")
        .args(keys.wtype_args())
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

fn paste_chord_xdotool(keys: PasteKeys) -> Result<()> {
    let status = Command::new("xdotool")
        .args(["key", keys.xdotool_key()])
        .status()
        .context("failed to spawn xdotool; install xdotool for X11 paste")?;
    if !status.success() {
        anyhow::bail!("xdotool exited with status {status}");
    }
    Ok(())
}

fn paste_chord_portal(keys: PasteKeys) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for portal call")?;
    runtime.block_on(paste_chord_portal_async(keys))
}

fn paste_text_wtype(text: &str) -> Result<()> {
    // `wtype -- <text>` types its arguments directly (handling shift / unicode
    // internally). The `--` terminates option parsing so a snippet beginning
    // with `-` doesn't get misinterpreted as a flag.
    let status = Command::new("wtype")
        .arg("--")
        .arg(text)
        .status()
        .context("failed to spawn wtype for type-mode paste")?;
    if !status.success() {
        anyhow::bail!("wtype exited with status {status}");
    }
    Ok(())
}

fn paste_text_xdotool(text: &str) -> Result<()> {
    // A small per-character delay avoids xdotool overrunning the X server's
    // input queue for long snippets.
    let status = Command::new("xdotool")
        .args(["type", "--delay", "5", "--", text])
        .status()
        .context("failed to spawn xdotool for type-mode paste")?;
    if !status.success() {
        anyhow::bail!("xdotool exited with status {status}");
    }
    Ok(())
}

fn paste_text_portal(text: &str) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for portal call")?;
    runtime.block_on(paste_text_portal_async(text))
}

async fn paste_chord_portal_async(keys: PasteKeys) -> Result<()> {
    use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
    use ashpd::desktop::PersistMode;

    let (modifiers, main_key) = keys.keysyms();

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

    // Press modifiers in order, press+release the main key, then release
    // modifiers in reverse order. A small inter-call pause keeps mutter
    // from coalescing/reordering the events under fast async chains.
    let step = Duration::from_millis(10);
    for &m in modifiers {
        proxy
            .notify_keyboard_keysym(&session, m, KeyState::Pressed)
            .await
            .with_context(|| format!("portal modifier keysym {m:#x} press failed"))?;
        thread::sleep(step);
    }
    proxy
        .notify_keyboard_keysym(&session, main_key, KeyState::Pressed)
        .await
        .with_context(|| format!("portal main keysym {main_key:#x} press failed"))?;
    thread::sleep(step);
    proxy
        .notify_keyboard_keysym(&session, main_key, KeyState::Released)
        .await
        .with_context(|| format!("portal main keysym {main_key:#x} release failed"))?;
    thread::sleep(step);
    for &m in modifiers.iter().rev() {
        proxy
            .notify_keyboard_keysym(&session, m, KeyState::Released)
            .await
            .with_context(|| format!("portal modifier keysym {m:#x} release failed"))?;
        thread::sleep(step);
    }

    Ok(())
}

async fn paste_text_portal_async(text: &str) -> Result<()> {
    use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
    use ashpd::desktop::PersistMode;

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

    // Type each character. A short per-key pause matches what wtype/xdotool
    // do internally and keeps fast typers (and slow apps) in sync.
    let step = Duration::from_millis(3);
    for c in text.chars() {
        let keysym = char_to_keysym(c).ok_or_else(|| {
            anyhow!(
                "character {c:?} (U+{:04X}) not supported in type mode",
                c as u32
            )
        })?;
        proxy
            .notify_keyboard_keysym(&session, keysym, KeyState::Pressed)
            .await
            .with_context(|| format!("portal keysym {keysym:#x} press failed"))?;
        thread::sleep(step);
        proxy
            .notify_keyboard_keysym(&session, keysym, KeyState::Released)
            .await
            .with_context(|| format!("portal keysym {keysym:#x} release failed"))?;
        thread::sleep(step);
    }

    Ok(())
}

/// Map a character to its X11 keysym for type-mode synthesis. Returns
/// `None` for characters we won't try to type (non-ASCII, control chars
/// other than newline/tab).
///
/// We send the *shifted-form* keysym directly (e.g. 0x41 for `A`, 0x21 for
/// `!`) rather than emulating a Shift+key chord — most compositors interpret
/// the resulting key event as the literal character, sidestepping the
/// modifier-handling bugs that bite chord synthesis.
fn char_to_keysym(c: char) -> Option<i32> {
    match c {
        '\n' => Some(0xff0d), // Return
        '\t' => Some(0xff09), // Tab
        c if c.is_ascii() && (c as u32) >= 0x20 && (c as u32) <= 0x7e => Some(c as i32),
        _ => None,
    }
}

fn validate_typeable(text: &str) -> Result<()> {
    for (i, c) in text.chars().enumerate() {
        if char_to_keysym(c).is_none() {
            anyhow::bail!(
                "character {c:?} (U+{:04X}) at position {i} is not supported in type mode; \
                 use --paste-mode chord or remove the character from the snippet",
                c as u32
            );
        }
    }
    Ok(())
}

fn token_path() -> Result<PathBuf> {
    let dirs =
        BaseDirs::new().ok_or_else(|| anyhow!("could not determine user config directory"))?;
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

    #[test]
    fn parse_xdotool_window_id_trims_output() {
        assert_eq!(
            parse_xdotool_window_id(b"12345678\n"),
            Some("12345678".to_string())
        );
    }

    #[test]
    fn parse_xdotool_window_id_rejects_empty_output() {
        assert_eq!(parse_xdotool_window_id(b"  \n"), None);
    }

    // ----- PasteKeys ------------------------------------------------------

    #[test]
    fn paste_keys_ctrl_v_uses_only_control_modifier() {
        let (modifiers, key) = PasteKeys::CtrlV.keysyms();
        assert_eq!(modifiers, &[KEYSYM_CONTROL_L]);
        assert_eq!(key, KEYSYM_V);
    }

    #[test]
    fn paste_keys_ctrl_shift_v_includes_shift() {
        let (modifiers, key) = PasteKeys::CtrlShiftV.keysyms();
        assert_eq!(modifiers, &[KEYSYM_CONTROL_L, KEYSYM_SHIFT_L]);
        assert_eq!(key, KEYSYM_V);
    }

    #[test]
    fn paste_keys_wtype_args_match_combo() {
        assert_eq!(
            PasteKeys::CtrlV.wtype_args(),
            &["-M", "ctrl", "-k", "v", "-m", "ctrl"]
        );
        assert_eq!(
            PasteKeys::CtrlShiftV.wtype_args(),
            &["-M", "ctrl", "-M", "shift", "-k", "v", "-m", "shift", "-m", "ctrl"]
        );
    }

    #[test]
    fn paste_keys_xdotool_strings() {
        assert_eq!(PasteKeys::CtrlV.xdotool_key(), "ctrl+v");
        assert_eq!(PasteKeys::CtrlShiftV.xdotool_key(), "ctrl+shift+v");
    }

    // ----- PasteMode::Type: char_to_keysym + validate_typeable -----------

    #[test]
    fn keysym_for_ascii_letters() {
        assert_eq!(char_to_keysym('a'), Some(0x61));
        assert_eq!(char_to_keysym('z'), Some(0x7a));
        assert_eq!(char_to_keysym('A'), Some(0x41));
        assert_eq!(char_to_keysym('Z'), Some(0x5a));
    }

    #[test]
    fn keysym_for_ascii_digits_and_symbols() {
        assert_eq!(char_to_keysym('0'), Some(0x30));
        assert_eq!(char_to_keysym('9'), Some(0x39));
        assert_eq!(char_to_keysym('!'), Some(0x21));
        assert_eq!(char_to_keysym(' '), Some(0x20));
        assert_eq!(char_to_keysym('~'), Some(0x7e));
    }

    #[test]
    fn keysym_for_whitespace_specials() {
        assert_eq!(char_to_keysym('\n'), Some(0xff0d));
        assert_eq!(char_to_keysym('\t'), Some(0xff09));
    }

    #[test]
    fn keysym_rejects_unsupported_chars() {
        assert_eq!(char_to_keysym('é'), None);
        assert_eq!(char_to_keysym('🙂'), None);
        assert_eq!(char_to_keysym('\u{7f}'), None); // DEL
        assert_eq!(char_to_keysym('\u{1f}'), None); // unit separator
    }

    #[test]
    fn validate_typeable_accepts_pure_ascii() {
        validate_typeable("Hello, World!\nLine 2\twith tab.").unwrap();
    }

    #[test]
    fn validate_typeable_rejects_non_ascii_with_position() {
        let err = validate_typeable("Hello, café").unwrap_err().to_string();
        assert!(err.contains("'é'"), "{err}");
        // "Hello, café" — é is char index 10 (H=0, e=1, …, f=9, é=10).
        assert!(err.contains("position 10"), "{err}");
    }

    #[test]
    fn validate_typeable_accepts_empty() {
        validate_typeable("").unwrap();
    }
}
