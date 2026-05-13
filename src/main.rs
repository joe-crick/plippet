mod clipboard;
mod config;
#[cfg(feature = "gui")]
mod gui;
mod injector;
mod picker;
mod snippet;
mod tools;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::injector::{PasteBackend, PasteKeys, PasteMode};
use crate::picker::{PickerKind, PickerOption};
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
        #[arg(long, value_enum, default_value_t = PickerKind::Auto)]
        picker: PickerKind,

        #[arg(long)]
        paste: bool,

        #[arg(long, value_enum, default_value_t = PasteBackend::Auto)]
        paste_backend: PasteBackend,

        #[arg(long, value_enum, default_value_t = PasteKeys::CtrlV)]
        paste_keys: PasteKeys,

        #[arg(long, value_enum, default_value_t = PasteMode::Chord)]
        paste_mode: PasteMode,
    },
    List,
    Insert {
        key: String,

        #[arg(long)]
        paste: bool,

        #[arg(long, value_enum, default_value_t = PasteBackend::Auto)]
        paste_backend: PasteBackend,

        #[arg(long, value_enum, default_value_t = PasteKeys::CtrlV)]
        paste_keys: PasteKeys,

        #[arg(long, value_enum, default_value_t = PasteMode::Chord)]
        paste_mode: PasteMode,
    },
    Check {
        #[arg(long)]
        strict: bool,
    },
    /// Open the snippet manager GUI.
    #[cfg(feature = "gui")]
    Edit,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Pick {
            picker,
            paste,
            paste_backend,
            paste_keys,
            paste_mode,
        } => pick_command(picker, paste, paste_backend, paste_keys, paste_mode),
        Commands::List => list_command(),
        Commands::Insert {
            key,
            paste,
            paste_backend,
            paste_keys,
            paste_mode,
        } => insert_command(&key, paste, paste_backend, paste_keys, paste_mode),
        Commands::Check { strict } => check_command(strict),
        #[cfg(feature = "gui")]
        Commands::Edit => edit_command(),
    }
}

#[cfg(feature = "gui")]
fn edit_command() -> anyhow::Result<()> {
    let path = Config::default_path()?;
    gui::run(path)
}

fn load_config() -> Result<Config> {
    let config = Config::load_default()?;
    config.validate()?;
    Ok(config)
}

fn pick_command(
    kind: PickerKind,
    paste: bool,
    backend: PasteBackend,
    keys: PasteKeys,
    mode: PasteMode,
) -> Result<()> {
    let config = load_config()?;

    let resolved_kind = match kind {
        PickerKind::Auto => picker::auto_pick()?,
        explicit => explicit,
    };

    let options: Vec<PickerOption> = config
        .snippet
        .iter()
        .map(|s| PickerOption {
            label: format!("{}\t{}", s.key, s.name),
        })
        .collect();

    let target = if paste {
        injector::capture_target(backend)?
    } else {
        None
    };

    let Some(selected_key) = picker::pick_with(resolved_kind, &options)? else {
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
        injector::paste(backend, keys, mode, target.as_ref(), &text)?;
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

fn insert_command(
    key: &str,
    paste: bool,
    backend: PasteBackend,
    keys: PasteKeys,
    mode: PasteMode,
) -> Result<()> {
    let config = load_config()?;
    let target = if paste {
        injector::capture_target(backend)?
    } else {
        None
    };

    let selected = config
        .snippet
        .iter()
        .find(|s| s.key == key)
        .with_context(|| format!("snippet not found: {key}"))?;

    let text = resolve_snippet(selected)?;
    clipboard::copy_to_clipboard(&text)?;

    if paste {
        injector::paste(backend, keys, mode, target.as_ref(), &text)?;
    }

    Ok(())
}

fn check_command(strict: bool) -> Result<()> {
    let path = Config::default_path()?;
    let config = load_config()?;

    let session = tools::session_kind();
    println!("session: {session}");
    println!(
        "config ok: {} snippets ({})",
        config.snippet.len(),
        path.display()
    );

    let (paste_resolved, paste_reason) = injector::describe_auto();
    println!("paste backend (auto resolves to): {paste_resolved:?}  [{paste_reason}]");
    let (picker_resolved, picker_reason) = picker::describe_auto_picker();
    let picker_label = match picker_resolved {
        Some(k) => format!("{k:?}"),
        None => "<none installed>".to_string(),
    };
    println!("picker (auto resolves to): {picker_label}  [{picker_reason}]");

    let required_missing = report_tools("required tools", tools::required_for(session));
    report_tools("optional tools", tools::OPTIONAL);

    if strict && !required_missing.is_empty() {
        anyhow::bail!("required tools missing: {}", required_missing.join(", "));
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
