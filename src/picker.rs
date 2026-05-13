use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PickerKind {
    Fuzzel,
    Wofi,
    Rofi,
    Bemenu,
}

#[derive(Debug, Clone)]
pub struct PickerOption {
    pub label: String,
}

pub trait Picker {
    fn pick(&self, options: &[PickerOption]) -> Result<Option<String>>;
}

pub struct ExternalPicker {
    kind: PickerKind,
}

impl ExternalPicker {
    pub fn new(kind: PickerKind) -> Self {
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

        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if selected.is_empty() {
            return Ok(None);
        }

        let key = selected
            .split_once('\t')
            .map(|(key, _)| key)
            .unwrap_or(selected.as_str())
            .to_string();

        Ok(Some(key))
    }
}
