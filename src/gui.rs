use std::path::PathBuf;

use anyhow::Result;
use eframe::egui;

use crate::config::{Config, SnippetConfig};

pub fn run(path: PathBuf) -> Result<()> {
    let app = SnippetApp::load(path);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("plippet — snippet manager")
            .with_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "plippet",
        options,
        Box::new(move |cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI failed: {e}"))
}

fn apply_theme(ctx: &egui::Context) {
    // Day-mode theme.
    ctx.set_visuals(egui::Visuals::light());

    // Bump the default text sizes so the UI is comfortable to read.
    // egui's defaults are quite small (Body 12.5, Heading 18); these are
    // ~25–30% larger.
    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(15.0, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(13.0, FontFamily::Proportional)),
        ]
        .into();
        // Give the buttons a bit more padding so they don't look cramped at
        // the larger font size.
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    });
}

// Status colors picked for contrast against the light theme background.
const COLOR_OK: egui::Color32 = egui::Color32::from_rgb(0, 110, 0);
const COLOR_ERR: egui::Color32 = egui::Color32::from_rgb(170, 0, 0);
const COLOR_DIRTY: egui::Color32 = egui::Color32::from_rgb(180, 110, 0);

const NEW_KEY_BASE: &str = "new";

fn next_new_key(existing: &[String]) -> String {
    let used: std::collections::HashSet<&str> = existing.iter().map(|s| s.as_str()).collect();
    if !used.contains(NEW_KEY_BASE) {
        return NEW_KEY_BASE.to_string();
    }
    for n in 2u32.. {
        let candidate = format!("{NEW_KEY_BASE}-{n}");
        if !used.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("u32 overflow before finding a free key — that's a lot of snippets")
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum DraftKind {
    Body,
    Command,
}

#[derive(Clone)]
struct SnippetDraft {
    key: String,
    name: String,
    kind: DraftKind,
    body: String,
    command: String,
}

impl SnippetDraft {
    fn from_config(s: &SnippetConfig) -> Self {
        let (kind, body, command) = match (&s.body, &s.command) {
            (Some(b), _) => (DraftKind::Body, b.clone(), String::new()),
            (_, Some(c)) => (DraftKind::Command, String::new(), c.clone()),
            _ => (DraftKind::Body, String::new(), String::new()),
        };
        SnippetDraft {
            key: s.key.clone(),
            name: s.name.clone(),
            kind,
            body,
            command,
        }
    }

    fn to_config(&self) -> SnippetConfig {
        SnippetConfig {
            key: self.key.clone(),
            name: self.name.clone(),
            body: matches!(self.kind, DraftKind::Body).then(|| self.body.clone()),
            command: matches!(self.kind, DraftKind::Command).then(|| self.command.clone()),
        }
    }

    fn new_unique(existing_keys: &[String]) -> Self {
        let key = next_new_key(existing_keys);
        let name = key.clone();
        SnippetDraft {
            key,
            name,
            kind: DraftKind::Body,
            body: String::new(),
            command: String::new(),
        }
    }
}

#[derive(Default, Clone)]
enum Modal {
    #[default]
    None,
    ConfirmDelete(usize),
    ConfirmRevert,
    ConfirmClose,
}

enum Status {
    Idle,
    Saved,
    Error(String),
}

struct SnippetApp {
    path: PathBuf,
    snippets: Vec<SnippetDraft>,
    selected: Option<usize>,
    dirty: bool,
    status: Status,
    modal: Modal,
    confirmed_close: bool,
}

impl SnippetApp {
    fn load(path: PathBuf) -> Self {
        let (snippets, status) = match Config::load_from(&path) {
            Ok(cfg) => (
                cfg.snippet.iter().map(SnippetDraft::from_config).collect(),
                Status::Idle,
            ),
            Err(e) => (Vec::new(), Status::Error(format!("{e:#}"))),
        };
        let selected = if snippets.is_empty() { None } else { Some(0) };
        SnippetApp {
            path,
            snippets,
            selected,
            dirty: false,
            status,
            modal: Modal::None,
            confirmed_close: false,
        }
    }

    fn current_config(&self) -> Config {
        Config {
            snippet: self.snippets.iter().map(|d| d.to_config()).collect(),
        }
    }

    fn validate(&self) -> Result<()> {
        self.current_config().validate()
    }

    fn save(&mut self) {
        let cfg = self.current_config();
        if let Err(e) = cfg.validate() {
            self.status = Status::Error(format!("not saved: {e}"));
            return;
        }
        match cfg.save_to(&self.path) {
            Ok(()) => {
                self.dirty = false;
                self.status = Status::Saved;
            }
            Err(e) => {
                self.status = Status::Error(format!("save failed: {e:#}"));
            }
        }
    }

    fn revert(&mut self) {
        let reloaded = Self::load(self.path.clone());
        self.snippets = reloaded.snippets;
        self.selected = reloaded.selected;
        self.dirty = false;
        self.status = reloaded.status;
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        if matches!(self.status, Status::Saved) {
            self.status = Status::Idle;
        }
    }

    fn add_snippet(&mut self) {
        let keys: Vec<String> = self.snippets.iter().map(|d| d.key.clone()).collect();
        self.snippets.push(SnippetDraft::new_unique(&keys));
        self.selected = Some(self.snippets.len() - 1);
        self.mark_dirty();
    }

    fn delete_snippet(&mut self, idx: usize) {
        if idx >= self.snippets.len() {
            return;
        }
        self.snippets.remove(idx);
        match self.selected {
            Some(sel) if sel == idx => self.selected = None,
            Some(sel) if sel > idx => self.selected = Some(sel - 1),
            _ => {}
        }
        self.mark_dirty();
    }
}

impl eframe::App for SnippetApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Close-while-dirty interception.
        if !self.confirmed_close
            && self.dirty
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.modal = Modal::ConfirmClose;
        }

        // Ctrl+S / Cmd+S to save (command == ctrl on Linux/Windows, cmd on macOS).
        let save_chord =
            ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S));
        if save_chord && self.validate().is_ok() {
            self.save();
        }

        self.render_toolbar(ctx);
        self.render_status_bar(ctx);
        self.render_list(ctx);
        self.render_editor(ctx);
        self.render_modal(ctx);
    }
}

impl SnippetApp {
    fn render_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let valid = self.validate().is_ok();
                if ui
                    .add_enabled(valid, egui::Button::new("💾 Save"))
                    .on_hover_text("Save snippets to disk (Ctrl+S)")
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .button("➕ Add snippet")
                    .on_hover_text("Add a new snippet with a placeholder key")
                    .clicked()
                {
                    self.add_snippet();
                }
                if ui
                    .button("↻ Revert")
                    .on_hover_text("Reload snippets from disk")
                    .clicked()
                {
                    if self.dirty {
                        self.modal = Modal::ConfirmRevert;
                    } else {
                        self.revert();
                    }
                }
                ui.separator();
                if self.dirty {
                    ui.colored_label(COLOR_DIRTY, "● Unsaved changes");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(self.path.display().to_string())
                            .monospace()
                            .weak(),
                    );
                });
            });
            ui.add_space(4.0);
        });
    }

    fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(2.0);
            match &self.status {
                Status::Saved => {
                    ui.colored_label(COLOR_OK, "✓ Saved");
                }
                Status::Error(msg) => {
                    ui.colored_label(COLOR_ERR, format!("✗ {msg}"));
                }
                Status::Idle => match self.validate() {
                    Ok(()) => {
                        let n = self.snippets.len();
                        let s = if n == 1 { "" } else { "s" };
                        ui.colored_label(COLOR_OK, format!("✓ {n} snippet{s}, valid"));
                    }
                    Err(e) => {
                        ui.colored_label(COLOR_ERR, format!("✗ {e}"));
                    }
                },
            }
            ui.add_space(2.0);
        });
    }

    fn render_list(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("snippet_list")
            .default_width(220.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.heading("Snippets");
                ui.separator();

                let mut delete_request: Option<usize> = None;
                let mut select_request: Option<usize> = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, snippet) in self.snippets.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let display = if snippet.key.is_empty() {
                                "(empty key)".to_string()
                            } else {
                                snippet.key.clone()
                            };
                            let selected = self.selected == Some(i);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("✕")
                                        .on_hover_text("Delete snippet")
                                        .clicked()
                                    {
                                        delete_request = Some(i);
                                    }
                                    if ui
                                        .add_sized(
                                            [ui.available_width(), 0.0],
                                            egui::SelectableLabel::new(selected, display),
                                        )
                                        .clicked()
                                    {
                                        select_request = Some(i);
                                    }
                                },
                            );
                        });
                    }
                });

                if let Some(i) = select_request {
                    self.selected = Some(i);
                }
                if let Some(i) = delete_request {
                    self.modal = Modal::ConfirmDelete(i);
                }
            });
    }

    fn render_editor(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(idx) = self.selected else {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    if self.snippets.is_empty() {
                        ui.label("No snippets yet. Click ➕ Add snippet to create one.");
                    } else {
                        ui.label("Select a snippet on the left to edit it.");
                    }
                });
                return;
            };
            if idx >= self.snippets.len() {
                self.selected = None;
                return;
            }

            let mut changed = false;
            // Scoped borrow so we can call self.mark_dirty() after.
            {
                let snippet = &mut self.snippets[idx];

                ui.add_space(4.0);
                let header = if snippet.key.is_empty() {
                    "(empty)".to_string()
                } else {
                    snippet.key.clone()
                };
                ui.heading(format!("Editing: {header}"));
                ui.separator();

                egui::Grid::new("editor_fields")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Key:");
                        changed |= ui.text_edit_singleline(&mut snippet.key).changed();
                        ui.end_row();

                        ui.label("Name:");
                        changed |= ui.text_edit_singleline(&mut snippet.name).changed();
                        ui.end_row();

                        ui.label("Kind:");
                        ui.horizontal(|ui| {
                            changed |= ui
                                .radio_value(&mut snippet.kind, DraftKind::Body, "Body")
                                .changed();
                            changed |= ui
                                .radio_value(
                                    &mut snippet.kind,
                                    DraftKind::Command,
                                    "Command",
                                )
                                .changed();
                        });
                        ui.end_row();
                    });

                ui.add_space(8.0);

                match snippet.kind {
                    DraftKind::Body => {
                        ui.label("Body (copied verbatim to clipboard):");
                        let resp = ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::multiline(&mut snippet.body)
                                .desired_rows(12)
                                .font(egui::TextStyle::Monospace),
                        );
                        changed |= resp.changed();
                    }
                    DraftKind::Command => {
                        ui.label("Command (sh -c; stdout copied, trailing newlines trimmed):");
                        let resp = ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::multiline(&mut snippet.command)
                                .desired_rows(12)
                                .font(egui::TextStyle::Monospace),
                        );
                        changed |= resp.changed();
                    }
                }
            }

            if changed {
                self.mark_dirty();
            }
        });
    }

    fn render_modal(&mut self, ctx: &egui::Context) {
        let current = std::mem::replace(&mut self.modal, Modal::None);
        let mut next = Modal::None;

        match current {
            Modal::None => return,
            Modal::ConfirmDelete(idx) => {
                let key = self
                    .snippets
                    .get(idx)
                    .map(|s| s.key.clone())
                    .unwrap_or_default();
                egui::Window::new("Delete snippet?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Delete snippet '{}'?", if key.is_empty() { "(empty key)" } else { &key }));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let delete = ui.button("Delete").clicked();
                            let cancel = ui.button("Cancel").clicked();
                            if delete {
                                self.delete_snippet(idx);
                            } else if !cancel {
                                next = Modal::ConfirmDelete(idx);
                            }
                        });
                    });
            }
            Modal::ConfirmRevert => {
                egui::Window::new("Discard changes?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("Discard unsaved changes and reload from disk?");
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let discard = ui.button("Discard & reload").clicked();
                            let cancel = ui.button("Cancel").clicked();
                            if discard {
                                self.revert();
                            } else if !cancel {
                                next = Modal::ConfirmRevert;
                            }
                        });
                    });
            }
            Modal::ConfirmClose => {
                let valid = self.validate().is_ok();
                let mut close_now = false;
                egui::Window::new("Unsaved changes")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("You have unsaved changes.");
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let save_quit = ui
                                .add_enabled(valid, egui::Button::new("Save & quit"))
                                .clicked();
                            let discard_quit = ui.button("Discard & quit").clicked();
                            let cancel = ui.button("Cancel").clicked();

                            if save_quit {
                                self.save();
                                if !self.dirty {
                                    self.confirmed_close = true;
                                    close_now = true;
                                } else {
                                    // save failed; keep modal open
                                    next = Modal::ConfirmClose;
                                }
                            } else if discard_quit {
                                self.confirmed_close = true;
                                close_now = true;
                            } else if !cancel {
                                next = Modal::ConfirmClose;
                            }
                        });
                    });
                if close_now {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        self.modal = next;
    }
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;

    #[test]
    fn next_new_key_when_no_existing() {
        assert_eq!(next_new_key(&[]), "new");
    }

    #[test]
    fn next_new_key_when_new_taken() {
        assert_eq!(next_new_key(&["new".to_string()]), "new-2");
    }

    #[test]
    fn next_new_key_when_new_and_new_2_taken() {
        assert_eq!(
            next_new_key(&["new".to_string(), "new-2".to_string()]),
            "new-3"
        );
    }

    #[test]
    fn next_new_key_skips_unrelated_keys() {
        assert_eq!(
            next_new_key(&["sig".to_string(), "addr".to_string()]),
            "new"
        );
    }

    #[test]
    fn next_new_key_fills_holes() {
        // `new` and `new-3` are taken; `new-2` is free → returns `new-2`.
        assert_eq!(
            next_new_key(&["new".to_string(), "new-3".to_string()]),
            "new-2"
        );
    }

    #[test]
    fn round_trip_body_snippet() {
        let s = SnippetConfig {
            key: "sig".into(),
            name: "Signoff".into(),
            body: Some("Best,\nJ\n".into()),
            command: None,
        };
        let draft = SnippetDraft::from_config(&s);
        assert_eq!(draft.kind, DraftKind::Body);
        assert_eq!(draft.body, "Best,\nJ\n");
        assert!(
            draft.command.is_empty(),
            "command buffer should be empty for body-only snippet"
        );

        let back = draft.to_config();
        assert_eq!(back.key, "sig");
        assert_eq!(back.body.as_deref(), Some("Best,\nJ\n"));
        assert_eq!(back.command, None);
    }

    #[test]
    fn round_trip_command_snippet() {
        let s = SnippetConfig {
            key: "today".into(),
            name: "Date".into(),
            body: None,
            command: Some("date +%Y-%m-%d".into()),
        };
        let draft = SnippetDraft::from_config(&s);
        assert_eq!(draft.kind, DraftKind::Command);
        assert_eq!(draft.command, "date +%Y-%m-%d");
        assert!(
            draft.body.is_empty(),
            "body buffer should be empty for command-only snippet"
        );

        let back = draft.to_config();
        assert_eq!(back.command.as_deref(), Some("date +%Y-%m-%d"));
        assert_eq!(back.body, None);
    }

    #[test]
    fn new_unique_creates_a_valid_draft() {
        let draft = SnippetDraft::new_unique(&[]);
        let cfg = Config {
            snippet: vec![draft.to_config()],
        };
        cfg.validate()
            .expect("a freshly-added snippet should pass validation");
    }

    #[test]
    fn new_unique_does_not_collide_with_existing_keys() {
        let existing = vec!["new".to_string(), "new-2".to_string()];
        let draft = SnippetDraft::new_unique(&existing);
        assert_eq!(draft.key, "new-3");
    }

    #[test]
    fn kind_toggle_does_not_corrupt_saved_config() {
        let s = SnippetConfig {
            key: "k".into(),
            name: "n".into(),
            body: Some("hello".into()),
            command: None,
        };
        let mut draft = SnippetDraft::from_config(&s);

        // Switch to command, type something, serialize → only command present.
        draft.kind = DraftKind::Command;
        draft.command.push_str("echo hi");
        let cfg = draft.to_config();
        assert_eq!(cfg.command.as_deref(), Some("echo hi"));
        assert_eq!(cfg.body, None);

        // Switch back to body → the original "hello" buffer survives.
        draft.kind = DraftKind::Body;
        let cfg = draft.to_config();
        assert_eq!(cfg.body.as_deref(), Some("hello"));
        assert_eq!(cfg.command, None);
    }
}
