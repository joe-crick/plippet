use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use clap::ValueEnum;

use crate::tools::{is_gnome_or_kde_desktop, is_on_path, session_kind, SessionKind};

#[cfg(feature = "gui")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "gui")]
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PickerKind {
    /// Heuristic — see `picker_preferences` for the resolution order.
    Auto,
    /// Built-in egui-based picker. Works on any compositor (Wayland or X11)
    /// because it doesn't depend on `wlr-layer-shell`. Slower cold-start
    /// than external pickers but categorically reliable.
    #[cfg(feature = "gui")]
    Builtin,
    Fuzzel,
    Wofi,
    Rofi,
    Bemenu,
}

impl PickerKind {
    /// Binary name on `$PATH`, or `None` for variants that aren't external
    /// tools (`Auto`, `Builtin`).
    pub fn binary(self) -> Option<&'static str> {
        match self {
            PickerKind::Auto => None,
            #[cfg(feature = "gui")]
            PickerKind::Builtin => None,
            PickerKind::Fuzzel => Some("fuzzel"),
            PickerKind::Wofi => Some("wofi"),
            PickerKind::Rofi => Some("rofi"),
            PickerKind::Bemenu => Some("bemenu"),
        }
    }

    /// True if this variant doesn't need anything on `$PATH` — used by the
    /// auto-resolver as an unconditional "yes, this one works" marker.
    fn is_always_available(self) -> bool {
        #[cfg(feature = "gui")]
        {
            return matches!(self, PickerKind::Builtin);
        }
        #[allow(unreachable_code)]
        false
    }
}

#[derive(Debug, Clone)]
pub struct PickerOption {
    pub label: String,
}

pub trait Picker {
    fn pick(&self, options: &[PickerOption]) -> Result<Option<String>>;
}

/// Dispatch entry point. Resolve `Auto` via `auto_pick()` first; this
/// function panics if asked to pick with `PickerKind::Auto`.
pub fn pick_with(kind: PickerKind, options: &[PickerOption]) -> Result<Option<String>> {
    match kind {
        PickerKind::Auto => unreachable!("resolve Auto via auto_pick() before calling pick_with"),
        #[cfg(feature = "gui")]
        PickerKind::Builtin => BuiltinPicker.pick(options),
        external => ExternalPicker::new(external).pick(options),
    }
}

pub struct ExternalPicker {
    kind: PickerKind,
}

impl ExternalPicker {
    /// `kind` must be a concrete external picker (not `Auto` or `Builtin`).
    pub fn new(kind: PickerKind) -> Self {
        debug_assert!(kind.binary().is_some(), "ExternalPicker requires an external-tool variant");
        Self { kind }
    }

    fn command(&self) -> Command {
        match self.kind {
            PickerKind::Fuzzel => {
                let mut cmd = Command::new("fuzzel");
                cmd.arg("--dmenu");
                cmd
            }
            PickerKind::Wofi => {
                let mut cmd = Command::new("wofi");
                cmd.arg("--dmenu");
                cmd
            }
            PickerKind::Rofi => {
                let mut cmd = Command::new("rofi");
                cmd.arg("-dmenu");
                cmd
            }
            PickerKind::Bemenu => Command::new("bemenu"),
            PickerKind::Auto => unreachable!("Auto should have been resolved earlier"),
            #[cfg(feature = "gui")]
            PickerKind::Builtin => unreachable!("Builtin is dispatched via BuiltinPicker"),
        }
    }
}

impl Picker for ExternalPicker {
    fn pick(&self, options: &[PickerOption]) -> Result<Option<String>> {
        let input = options
            .iter()
            .map(|option| option.label.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let mut child = self
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn picker {:?}", self.kind))?;

        {
            let stdin = child
                .stdin
                .as_mut()
                .context("failed to open picker stdin")?;
            stdin.write_all(input.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_selected_row(&stdout))
    }
}

/// Resolve the picker for this environment, preferring tools that actually
/// work on the current compositor and that are installed.
pub fn auto_pick() -> Result<PickerKind> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    let session = session_kind();
    let prefs = picker_preferences(session, desktop.as_deref());
    match resolve_auto_picker(&prefs, |bin| is_on_path(bin)) {
        Some(kind) => Ok(kind),
        None => {
            let names: Vec<&str> = prefs.iter().filter_map(|p| p.binary()).collect();
            anyhow::bail!(
                "no compatible picker on $PATH; tried (in priority order): {}",
                names.join(", ")
            )
        }
    }
}

/// Analog of `crate::injector::describe_auto` — what `--picker auto` would
/// pick right now, plus a one-line reason.
pub fn describe_auto_picker() -> (Option<PickerKind>, String) {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    let session = session_kind();
    let prefs = picker_preferences(session, desktop.as_deref());
    let resolved = resolve_auto_picker(&prefs, |bin| is_on_path(bin));
    let reason = preference_reason(session, desktop.as_deref());
    (resolved, reason)
}

/// Pure priority list: which pickers we'd *prefer* on this environment, in
/// order. Doesn't consult `$PATH`.
fn picker_preferences(session: SessionKind, desktop: Option<&str>) -> Vec<PickerKind> {
    let gnome_or_kde_wayland =
        session == SessionKind::Wayland && is_gnome_or_kde_desktop(desktop);
    let mut prefs: Vec<PickerKind> = Vec::new();

    if gnome_or_kde_wayland {
        // GNOME / KDE Wayland: `wlr-layer-shell` isn't available, so
        // fuzzel/wofi/bemenu's Wayland mode can't draw. `rofi` *might* be
        // plain X11 rofi (works via XWayland) OR rofi-wayland (the lbonn
        // fork — also needs wlr-layer-shell, also broken). We can't tell
        // them apart by binary name, so the built-in goes first to
        // guarantee a working picker.
        #[cfg(feature = "gui")]
        prefs.push(PickerKind::Builtin);
        prefs.push(PickerKind::Rofi);
    } else if session == SessionKind::X11 {
        // Pure X11 — plain rofi (and any other X11 launcher) works
        // natively, no rofi-wayland mismatch risk.
        prefs.push(PickerKind::Rofi);
        #[cfg(feature = "gui")]
        prefs.push(PickerKind::Builtin);
    } else {
        // wlroots-style Wayland (Sway, Hyprland, river, labwc, …) or
        // unknown Wayland — external pickers work natively here.
        prefs.extend([
            PickerKind::Fuzzel,
            PickerKind::Wofi,
            PickerKind::Bemenu,
            PickerKind::Rofi,
        ]);
        #[cfg(feature = "gui")]
        prefs.push(PickerKind::Builtin);
    }

    prefs
}

/// Pure resolver: pick the first preference that's either always-available
/// (built-in) or whose binary is on `$PATH` (as reported by `is_installed`).
fn resolve_auto_picker<F: Fn(&str) -> bool>(
    preferences: &[PickerKind],
    is_installed: F,
) -> Option<PickerKind> {
    for kind in preferences {
        if kind.is_always_available() {
            return Some(*kind);
        }
        if let Some(bin) = kind.binary() {
            if is_installed(bin) {
                return Some(*kind);
            }
        }
    }
    None
}

fn preference_reason(session: SessionKind, desktop: Option<&str>) -> String {
    let gnome_or_kde_wayland =
        session == SessionKind::Wayland && is_gnome_or_kde_desktop(desktop);
    if gnome_or_kde_wayland {
        let d = desktop.unwrap_or("");
        format!(
            "GNOME/KDE Wayland (no wlr-layer-shell; rofi may be rofi-wayland) \
             → builtin first, XDG_CURRENT_DESKTOP={d}"
        )
    } else if session == SessionKind::X11 {
        "X11 session → rofi first, builtin as fallback".to_string()
    } else {
        "Wayland (wlroots-style) → fuzzel/wofi/bemenu first, builtin as fallback"
            .to_string()
    }
}

/// Extracts the selected key from a picker's stdout. Returns `None` for
/// cancellation (empty / whitespace-only output). If the selected row has the
/// expected `"<key>\t<name>"` shape we return just the key; otherwise we fall
/// back to the entire trimmed row so an unexpected picker layout still
/// produces *something* usable.
fn parse_selected_row(stdout: &str) -> Option<String> {
    let selected = stdout.trim();
    if selected.is_empty() {
        return None;
    }
    let key = selected
        .split_once('\t')
        .map(|(k, _)| k)
        .unwrap_or(selected);
    Some(key.to_string())
}

// ---------------------------------------------------------------------------
// Built-in egui picker
// ---------------------------------------------------------------------------

#[cfg(feature = "gui")]
pub struct BuiltinPicker;

#[cfg(feature = "gui")]
impl Picker for BuiltinPicker {
    fn pick(&self, options: &[PickerOption]) -> Result<Option<String>> {
        let selected: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let app = BuiltinPickerApp::new(options.to_vec(), selected.clone());
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("plippet")
                .with_inner_size([560.0, 380.0])
                .with_resizable(false),
            ..Default::default()
        };
        eframe::run_native(
            "plippet picker",
            native_options,
            Box::new(move |cc| {
                apply_picker_theme(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
        )
        .map_err(|e| anyhow::anyhow!("builtin picker failed: {e}"))?;
        let result = selected.lock().unwrap().take();
        Ok(result)
    }
}

#[cfg(feature = "gui")]
fn apply_picker_theme(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::light());
    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(15.0, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(13.0, FontFamily::Proportional)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    });
}

#[cfg(feature = "gui")]
struct BuiltinPickerApp {
    options: Vec<PickerOption>,
    filter: String,
    selected_index: usize,
    result: Arc<Mutex<Option<String>>>,
    focus_set: bool,
}

#[cfg(feature = "gui")]
impl BuiltinPickerApp {
    fn new(options: Vec<PickerOption>, result: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            options,
            filter: String::new(),
            selected_index: 0,
            result,
            focus_set: false,
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        filter_options(&self.options, &self.filter)
    }

    fn commit(&self, filtered: &[usize], ctx: &egui::Context) {
        if let Some(&idx) = filtered.get(self.selected_index) {
            let label = &self.options[idx].label;
            let key = parse_selected_row(label).unwrap_or_else(|| label.clone());
            *self.result.lock().unwrap() = Some(key);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(feature = "gui")]
fn filter_options(options: &[PickerOption], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..options.len()).collect();
    }
    options
        .iter()
        .enumerate()
        .filter_map(|(i, opt)| {
            if opt.label.to_lowercase().contains(&q) {
                Some(i)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(feature = "gui")]
fn format_row(label: &str) -> String {
    // Tabs render as zero-width in egui by default; substitute spaces so the
    // key/name columns are visually separated.
    label.replace('\t', "   ")
}

#[cfg(feature = "gui")]
impl eframe::App for BuiltinPickerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let filtered = self.filtered_indices();
        if self.selected_index >= filtered.len() {
            self.selected_index = filtered.len().saturating_sub(1);
        }

        let (esc, enter, down, up) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
            )
        });
        if esc {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if enter && !filtered.is_empty() {
            self.commit(&filtered, ctx);
            return;
        }
        if down && !filtered.is_empty() {
            self.selected_index =
                (self.selected_index + 1).min(filtered.len().saturating_sub(1));
        }
        if up && self.selected_index > 0 {
            self.selected_index -= 1;
        }

        let mut commit_now = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            let filter_response = ui.add_sized(
                [ui.available_width(), 0.0],
                egui::TextEdit::singleline(&mut self.filter).hint_text("Type to filter…"),
            );
            if !self.focus_set {
                filter_response.request_focus();
                self.focus_set = true;
            }
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                if filtered.is_empty() {
                    ui.weak("(no matching snippets)");
                    return;
                }
                for (display_idx, &orig_idx) in filtered.iter().enumerate() {
                    let label = &self.options[orig_idx].label;
                    let selected = display_idx == self.selected_index;
                    let response = ui.add_sized(
                        [ui.available_width(), 0.0],
                        egui::SelectableLabel::new(selected, format_row(label)),
                    );
                    if response.clicked() {
                        self.selected_index = display_idx;
                        commit_now = true;
                    }
                }
            });
        });
        if commit_now {
            self.commit(&filtered, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stdout_is_cancellation() {
        assert_eq!(parse_selected_row(""), None);
    }

    #[test]
    fn whitespace_only_is_cancellation() {
        assert_eq!(parse_selected_row("\n"), None);
        assert_eq!(parse_selected_row("   \n  \n"), None);
    }

    #[test]
    fn tab_separated_row_returns_key() {
        assert_eq!(
            parse_selected_row("sig\tEmail signoff"),
            Some("sig".to_string())
        );
    }

    #[test]
    fn trailing_newline_is_stripped() {
        assert_eq!(
            parse_selected_row("sig\tEmail signoff\n"),
            Some("sig".to_string())
        );
    }

    #[test]
    fn row_without_tab_falls_back_to_full_row() {
        assert_eq!(
            parse_selected_row("just-the-row\n"),
            Some("just-the-row".to_string())
        );
    }

    #[test]
    fn only_first_tab_is_used_as_separator() {
        assert_eq!(parse_selected_row("k\tone\ttwo"), Some("k".to_string()));
    }

    // --- picker_preferences ---------------------------------------------

    #[test]
    fn preferences_wlroots_lead_with_fuzzel() {
        let prefs = picker_preferences(SessionKind::Wayland, Some("sway"));
        assert_eq!(prefs[0], PickerKind::Fuzzel);
        // Rofi sits at the back of the external list, before any builtin
        // fallback that may follow.
        assert!(prefs.contains(&PickerKind::Rofi));
    }

    #[test]
    fn preferences_gnome_wayland_starts_with_builtin_when_gui_compiled() {
        let prefs = picker_preferences(SessionKind::Wayland, Some("GNOME"));
        #[cfg(feature = "gui")]
        assert_eq!(prefs[0], PickerKind::Builtin);
        // rofi is still in the list as a secondary option (it may be plain X11
        // rofi, which would work fine).
        assert!(prefs.contains(&PickerKind::Rofi));
    }

    #[test]
    fn preferences_kde_wayland_starts_with_builtin_when_gui_compiled() {
        let prefs = picker_preferences(SessionKind::Wayland, Some("KDE"));
        #[cfg(feature = "gui")]
        assert_eq!(prefs[0], PickerKind::Builtin);
        assert!(prefs.contains(&PickerKind::Rofi));
    }

    #[test]
    fn preferences_x11_starts_with_rofi() {
        let prefs = picker_preferences(SessionKind::X11, Some("XFCE"));
        assert_eq!(prefs[0], PickerKind::Rofi);
        #[cfg(feature = "gui")]
        assert!(prefs.contains(&PickerKind::Builtin));
    }

    #[test]
    fn preferences_unset_desktop_on_wayland_assumes_wlroots() {
        let prefs = picker_preferences(SessionKind::Wayland, None);
        assert_eq!(prefs[0], PickerKind::Fuzzel);
    }

    #[test]
    #[cfg(feature = "gui")]
    fn preferences_wlroots_ends_with_builtin_as_fallback() {
        let prefs = picker_preferences(SessionKind::Wayland, Some("sway"));
        assert_eq!(prefs.last(), Some(&PickerKind::Builtin));
    }

    // --- resolve_auto_picker --------------------------------------------

    #[test]
    fn resolve_picks_first_installed_preference() {
        let prefs = vec![
            PickerKind::Fuzzel,
            PickerKind::Wofi,
            PickerKind::Bemenu,
            PickerKind::Rofi,
        ];
        let resolved = resolve_auto_picker(&prefs, |bin| bin == "wofi");
        assert_eq!(resolved, Some(PickerKind::Wofi));
    }

    #[test]
    fn resolve_skips_uninstalled_higher_priorities() {
        let prefs = vec![
            PickerKind::Fuzzel,
            PickerKind::Wofi,
            PickerKind::Bemenu,
            PickerKind::Rofi,
        ];
        let resolved = resolve_auto_picker(&prefs, |bin| bin == "bemenu");
        assert_eq!(resolved, Some(PickerKind::Bemenu));
    }

    #[test]
    fn resolve_returns_none_when_nothing_installed_and_no_builtin() {
        let prefs = vec![PickerKind::Fuzzel, PickerKind::Rofi];
        let resolved = resolve_auto_picker(&prefs, |_| false);
        assert_eq!(resolved, None);
    }

    #[test]
    #[cfg(feature = "gui")]
    fn resolve_returns_builtin_when_no_external_picker_installed() {
        let prefs = picker_preferences(SessionKind::Wayland, Some("GNOME"));
        let resolved = resolve_auto_picker(&prefs, |_| false);
        assert_eq!(resolved, Some(PickerKind::Builtin));
    }

    #[test]
    #[cfg(feature = "gui")]
    fn resolve_prefers_builtin_over_rofi_on_gnome_wayland() {
        // The user's exact case: GNOME Wayland with `rofi` on PATH but it's
        // really rofi-wayland (still broken). We can't tell from the binary
        // name, so Builtin must win the order.
        let prefs = picker_preferences(SessionKind::Wayland, Some("GNOME-Classic:GNOME:"));
        let resolved = resolve_auto_picker(&prefs, |bin| bin == "rofi");
        assert_eq!(resolved, Some(PickerKind::Builtin));
    }

    #[test]
    #[cfg(feature = "gui")]
    fn resolve_prefers_external_over_builtin_on_wlroots() {
        let prefs = picker_preferences(SessionKind::Wayland, Some("sway"));
        let resolved = resolve_auto_picker(&prefs, |bin| bin == "fuzzel");
        assert_eq!(resolved, Some(PickerKind::Fuzzel));
    }

    #[test]
    #[cfg(feature = "gui")]
    fn resolve_prefers_rofi_over_builtin_on_x11() {
        let prefs = picker_preferences(SessionKind::X11, Some("XFCE"));
        let resolved = resolve_auto_picker(&prefs, |bin| bin == "rofi");
        assert_eq!(resolved, Some(PickerKind::Rofi));
    }

    // --- builtin picker filtering ---------------------------------------

    #[cfg(feature = "gui")]
    fn opt(label: &str) -> PickerOption {
        PickerOption {
            label: label.to_string(),
        }
    }

    #[test]
    #[cfg(feature = "gui")]
    fn filter_empty_query_returns_all_in_order() {
        let opts = vec![opt("sig\tSignoff"), opt("addr\tAddress")];
        assert_eq!(filter_options(&opts, ""), vec![0, 1]);
        assert_eq!(filter_options(&opts, "   "), vec![0, 1]);
    }

    #[test]
    #[cfg(feature = "gui")]
    fn filter_is_case_insensitive_substring() {
        let opts = vec![opt("sig\tSignoff"), opt("addr\tAddress")];
        assert_eq!(filter_options(&opts, "sIg"), vec![0]);
        assert_eq!(filter_options(&opts, "address"), vec![1]);
    }

    #[test]
    #[cfg(feature = "gui")]
    fn filter_matches_against_name_not_just_key() {
        let opts = vec![opt("sig\tEmail signoff"), opt("addr\tStreet address")];
        let matches = filter_options(&opts, "email");
        assert_eq!(matches, vec![0]);
        let matches = filter_options(&opts, "street");
        assert_eq!(matches, vec![1]);
    }

    #[test]
    #[cfg(feature = "gui")]
    fn filter_no_match_returns_empty() {
        let opts = vec![opt("sig\tSignoff")];
        assert_eq!(filter_options(&opts, "zzz"), Vec::<usize>::new());
    }

    #[test]
    #[cfg(feature = "gui")]
    fn format_row_substitutes_tabs_with_spaces() {
        assert_eq!(format_row("sig\tSignoff"), "sig   Signoff");
    }
}
