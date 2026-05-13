use std::env;
use std::path::Path;

pub const WAYLAND_REQUIRED: &[&str] = &["wl-copy"];
pub const X11_REQUIRED: &[&str] = &["xclip"];
pub const OPTIONAL: &[&str] = &["wtype", "xdotool", "fuzzel", "wofi", "rofi", "bemenu"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Wayland,
    X11,
}

impl std::fmt::Display for SessionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionKind::Wayland => write!(f, "wayland"),
            SessionKind::X11 => write!(f, "x11"),
        }
    }
}

pub fn session_kind() -> SessionKind {
    session_kind_from_env(
        env::var("XDG_SESSION_TYPE").ok().as_deref(),
        env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )
}

/// Pure session-kind heuristic so tests don't have to mutate process-global
/// env vars. Wayland wins if either `XDG_SESSION_TYPE=wayland` or
/// `WAYLAND_DISPLAY` is set to a non-empty value; otherwise we treat it as
/// X11. (Empty strings, which bash produces for `VAR= prog`, are treated as
/// absent.)
pub fn session_kind_from_env(
    session_type: Option<&str>,
    wayland_display: Option<&str>,
) -> SessionKind {
    let session_type = session_type.filter(|s| !s.is_empty());
    let wayland_display = wayland_display.filter(|s| !s.is_empty());
    let is_wayland = matches!(session_type, Some(s) if s.eq_ignore_ascii_case("wayland"))
        || wayland_display.is_some();
    if is_wayland {
        SessionKind::Wayland
    } else {
        SessionKind::X11
    }
}

pub fn required_for(session: SessionKind) -> &'static [&'static str] {
    match session {
        SessionKind::Wayland => WAYLAND_REQUIRED,
        SessionKind::X11 => X11_REQUIRED,
    }
}

/// True if `XDG_CURRENT_DESKTOP` looks like GNOME / KDE Plasma. Those
/// compositors don't expose `wlr-layer-shell` (so fuzzel/wofi/bemenu's
/// Wayland mode can't draw their overlay) and don't expose
/// `virtual-keyboard-unstable-v1` (so `wtype` doesn't work either).
pub fn is_gnome_or_kde_desktop(desktop: Option<&str>) -> bool {
    let d = desktop.unwrap_or("").to_lowercase();
    d.contains("gnome") || d.contains("kde") || d.contains("plasma") || d.contains("kwin")
}

pub fn is_on_path(bin: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(bin);
        candidate.is_file() && is_executable(&candidate)
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn finds_sh_on_path() {
        // /bin/sh is on every conformant Unix and is always on $PATH in CI.
        assert!(is_on_path("sh"));
    }

    #[test]
    fn rejects_nonexistent_binary() {
        assert!(!is_on_path(
            "plippet-definitely-not-a-real-binary-9z7q-x2"
        ));
    }

    #[test]
    #[cfg(unix)]
    fn rejects_non_executable_file_in_path() {
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("not-an-exe");
        fs::write(&bin_path, b"not really a binary").unwrap();
        // file exists but lacks +x — should NOT be reported as on PATH

        let original = env::var_os("PATH");
        let new_path = format!("{}:{}", dir.path().display(), env::var("PATH").unwrap_or_default());
        // SAFETY: tests run in the same process; we restore PATH before returning.
        // Cargo runs tests in parallel by default, so we accept the small race
        // window — the assertion holds regardless because we check by name and
        // the bogus binary is in a unique temp dir.
        unsafe { env::set_var("PATH", &new_path) };
        let found = is_on_path("not-an-exe");
        match original {
            Some(v) => unsafe { env::set_var("PATH", v) },
            None => unsafe { env::remove_var("PATH") },
        }

        assert!(!found, "non-executable file should not register as on PATH");
    }

    #[test]
    fn session_kind_wayland_via_session_type() {
        assert_eq!(
            session_kind_from_env(Some("wayland"), None),
            SessionKind::Wayland
        );
    }

    #[test]
    fn session_kind_wayland_via_session_type_uppercase() {
        // some environments set Wayland (capitalised); be lenient.
        assert_eq!(
            session_kind_from_env(Some("Wayland"), None),
            SessionKind::Wayland
        );
    }

    #[test]
    fn session_kind_wayland_via_display_only() {
        assert_eq!(
            session_kind_from_env(None, Some("wayland-0")),
            SessionKind::Wayland
        );
    }

    #[test]
    fn session_kind_x11_when_no_wayland_signals() {
        assert_eq!(
            session_kind_from_env(Some("x11"), None),
            SessionKind::X11
        );
        assert_eq!(session_kind_from_env(None, None), SessionKind::X11);
    }

    #[test]
    fn session_kind_treats_empty_strings_as_absent() {
        // `WAYLAND_DISPLAY= prog` in bash sets the var to empty — that
        // shouldn't fool the detector into thinking the session is Wayland.
        assert_eq!(
            session_kind_from_env(Some("x11"), Some("")),
            SessionKind::X11
        );
        assert_eq!(
            session_kind_from_env(Some(""), Some("")),
            SessionKind::X11
        );
        // …and an empty session_type shouldn't override a real WAYLAND_DISPLAY.
        assert_eq!(
            session_kind_from_env(Some(""), Some("wayland-0")),
            SessionKind::Wayland
        );
    }

    #[test]
    fn required_for_wayland_is_wl_copy() {
        assert_eq!(required_for(SessionKind::Wayland), &["wl-copy"]);
    }

    #[test]
    fn required_for_x11_is_xclip() {
        assert_eq!(required_for(SessionKind::X11), &["xclip"]);
    }

    #[test]
    fn is_gnome_or_kde_recognises_the_obvious_values() {
        assert!(is_gnome_or_kde_desktop(Some("GNOME")));
        assert!(is_gnome_or_kde_desktop(Some("GNOME-Classic:GNOME:")));
        assert!(is_gnome_or_kde_desktop(Some("KDE")));
        assert!(is_gnome_or_kde_desktop(Some("plasma")));
        assert!(is_gnome_or_kde_desktop(Some("KWin")));
    }

    #[test]
    fn is_gnome_or_kde_rejects_wlroots_compositors_and_unset() {
        assert!(!is_gnome_or_kde_desktop(Some("sway")));
        assert!(!is_gnome_or_kde_desktop(Some("Hyprland")));
        assert!(!is_gnome_or_kde_desktop(Some("XFCE")));
        assert!(!is_gnome_or_kde_desktop(Some("")));
        assert!(!is_gnome_or_kde_desktop(None));
    }

    #[test]
    #[cfg(unix)]
    fn finds_executable_file_in_path() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("fake-bin-xyz");
        fs::write(&bin_path, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin_path, perms).unwrap();

        let original = env::var_os("PATH");
        let new_path = format!("{}:{}", dir.path().display(), env::var("PATH").unwrap_or_default());
        unsafe { env::set_var("PATH", &new_path) };
        let found = is_on_path("fake-bin-xyz");
        match original {
            Some(v) => unsafe { env::set_var("PATH", v) },
            None => unsafe { env::remove_var("PATH") },
        }

        assert!(found, "executable file in PATH should be found");
    }
}
