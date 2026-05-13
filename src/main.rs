mod clipboard;
mod config;
mod injector;
mod picker;
mod snippet;
mod tools;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::injector::PasteBackend;
use crate::picker::{ExternalPicker, Picker, PickerKind, PickerOption};
use crate::snippet::resolve_snippet;

#[derive(Debug, Parser)]
#[command(name = "plippet")]
#[command(about = "A small Wayland-friendly hotkey snippet picker")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Pick {
        #[arg(long, value_enum, default_value_t = PickerKind::Fuzzel)]
        picker: PickerKind,

        #[arg(long)]
        paste: bool,

        #[arg(long, value_enum, default_value_t = PasteBackend::Auto)]
        paste_backend: PasteBackend,
    },
    List,
    Insert {
        key: String,

        #[arg(long)]
        paste: bool,

        #[arg(long, value_enum, default_value_t = PasteBackend::Auto)]
        paste_backend: PasteBackend,
    },
    Check {
        #[arg(long)]
        strict: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Pick {
            picker,
            paste,
            paste_backend,
        } => pick_command(picker, paste, paste_backend),
        Commands::List => list_command(),
        Commands::Insert {
            key,
            paste,
            paste_backend,
        } => insert_command(&key, paste, paste_backend),
        Commands::Check { strict } => check_command(strict),
    }
}

fn load_config() -> Result<Config> {
    let config = Config::load_default()?;
    config.validate()?;
    Ok(config)
}

fn pick_command(kind: PickerKind, paste: bool, backend: PasteBackend) -> Result<()> {
    let config = load_config()?;

    let options: Vec<PickerOption> = config
        .snippet
        .iter()
        .map(|s| PickerOption {
            label: format!("{}\t{}", s.key, s.name),
        })
        .collect();

    let picker = ExternalPicker::new(kind);
    let Some(selected_key) = picker.pick(&options)? else {
        return Ok(());
    };

    let selected = config
        .snippet
        .iter()
        .find(|s| s.key == selected_key)
        .with_context(|| format!("selected snippet key not found: {selected_key}"))?;

    let text = resolve_snippet(selected)?;
    clipboard::copy_to_clipboard(&text)?;

    if paste {
        injector::paste(backend)?;
    }

    Ok(())
}

fn list_command() -> Result<()> {
    let config = load_config()?;

    for snippet in &config.snippet {
        println!("{}\t{}", snippet.key, snippet.name);
    }

    Ok(())
}

fn insert_command(key: &str, paste: bool, backend: PasteBackend) -> Result<()> {
    let config = load_config()?;

    let selected = config
        .snippet
        .iter()
        .find(|s| s.key == key)
        .with_context(|| format!("snippet not found: {key}"))?;

    let text = resolve_snippet(selected)?;
    clipboard::copy_to_clipboard(&text)?;

    if paste {
        injector::paste(backend)?;
    }

    Ok(())
}

fn check_command(strict: bool) -> Result<()> {
    let path = Config::default_path()?;
    let config = load_config()?;

    println!(
        "config ok: {} snippets ({})",
        config.snippet.len(),
        path.display()
    );

    let (resolved, reason) = injector::describe_auto();
    println!("paste backend (auto resolves to): {resolved:?}  [{reason}]");

    let required_missing = report_tools("required tools", tools::REQUIRED);
    report_tools("optional tools", tools::OPTIONAL);

    if strict && !required_missing.is_empty() {
        anyhow::bail!(
            "required tools missing: {}",
            required_missing.join(", ")
        );
    }

    Ok(())
}

fn report_tools(header: &str, names: &[&str]) -> Vec<String> {
    println!("{header}:");
    let width = names.iter().map(|n| n.len()).max().unwrap_or(0);
    let mut missing = Vec::new();
    for name in names {
        if tools::is_on_path(name) {
            println!("  {name:<width$}  ok");
        } else {
            println!("  {name:<width$}  missing");
            missing.push((*name).to_string());
        }
    }
    missing
}
