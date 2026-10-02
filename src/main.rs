use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use wezurrection::launch_agent;
use wezurrection::session::{snapshot_from_raw, RawPane, Snapshot};
use wezurrection::storage;
use wezurrection::wezterm::WezTerm;

#[derive(Debug, Parser)]
#[command(
    name = "wezurrection",
    version,
    about = "Save and resurrect WezTerm sessions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Enable,
    Disable,
    Status,
    Save,
    Restore,
    Watch,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Enable => enable(),
        Command::Disable => disable(),
        Command::Status => status(),
        Command::Save => save(),
        Command::Restore => restore(),
        Command::Watch => watch(),
    }
}

fn enable() -> Result<()> {
    let executable =
        std::env::current_exe().context("could not determine wezurrection executable")?;
    launch_agent::enable(
        executable
            .to_str()
            .context("executable path is not valid UTF-8")?,
    )?;
    println!("wezurrection background agent enabled");
    Ok(())
}

fn disable() -> Result<()> {
    launch_agent::disable()?;
    println!("wezurrection background agent disabled");
    Ok(())
}

fn status() -> Result<()> {
    let snapshot = storage::read_snapshot()?;
    let agent = launch_agent::is_running().unwrap_or(false);
    println!("agent: {}", if agent { "running" } else { "stopped" });
    match snapshot {
        Some(snapshot) => println!(
            "snapshot: {} window(s), saved {}",
            snapshot.windows.len(),
            storage::snapshot_age(&snapshot)
        ),
        None => println!("snapshot: none"),
    }
    Ok(())
}

fn save() -> Result<()> {
    let snapshot = capture(&WezTerm::default(), &HashSet::new())?;
    storage::write_snapshot(&snapshot)?;
    println!("saved {} window(s)", snapshot.windows.len());
    Ok(())
}

fn restore() -> Result<()> {
    let snapshot = storage::read_snapshot()?.context("no saved snapshot exists")?;
    let home = dirs::home_dir().context("could not determine the home directory")?;
    WezTerm::default().restore(&snapshot, &home)?;
    println!("restored {} window(s)", snapshot.windows.len());
    Ok(())
}

fn watch() -> Result<()> {
    const POLL_INTERVAL: Duration = Duration::from_secs(1);
    const SAVE_INTERVAL: Duration = Duration::from_secs(60);

    let terminal = WezTerm::default();
    let home = dirs::home_dir().context("could not determine the home directory")?;
    let mut was_running = false;
    let mut observed_absence = false;
    let mut restored = false;
    let mut ignored_windows = HashSet::new();
    let mut last_fingerprint = None;
    let mut last_save = Instant::now() - SAVE_INTERVAL;

    loop {
        match terminal.list() {
            Ok(panes) if !panes.is_empty() => {
                if !was_running {
                    restored = !observed_absence;
                    ignored_windows.clear();
                }
                was_running = true;

                if !restored {
                    if let Some(snapshot) = storage::read_snapshot()? {
                        ignored_windows = panes.iter().map(|pane| pane.window_id).collect();
                        match terminal.restore(&snapshot, &home) {
                            Ok(()) if panes.len() == 1 => match terminal.list() {
                                Ok(current) => {
                                    if let Some(pane_id) = starter_pane_to_close(&panes, &current) {
                                        if let Err(error) = terminal.kill_pane(pane_id) {
                                            eprintln!("wezurrection: could not close starter window: {error:#}");
                                        }
                                    }
                                }
                                Err(error) => eprintln!(
                                    "wezurrection: could not inspect starter window: {error:#}"
                                ),
                            },
                            Ok(()) => {}
                            Err(error) => eprintln!("wezurrection: restore failed: {error:#}"),
                        }
                    }
                    restored = true;
                }

                if last_save.elapsed() >= SAVE_INTERVAL {
                    let snapshot = capture(&terminal, &ignored_windows)?;
                    let fingerprint = serde_json::to_string(&snapshot.windows)?;
                    if last_fingerprint.as_deref() != Some(fingerprint.as_str()) {
                        storage::write_snapshot(&snapshot)?;
                        last_fingerprint = Some(fingerprint);
                    }
                    last_save = Instant::now();
                }
            }
            Ok(_) | Err(_) => {
                was_running = false;
                observed_absence = true;
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn capture(terminal: &WezTerm, ignored_windows: &HashSet<u64>) -> Result<Snapshot> {
    let raw = terminal.list()?;
    snapshot_from_raw(&raw, ignored_windows, storage::unix_time())
}

fn starter_pane_to_close(initial: &[RawPane], current: &[RawPane]) -> Option<u64> {
    let [starter] = initial else {
        return None;
    };
    let mut panes = current
        .iter()
        .filter(|pane| pane.window_id == starter.window_id);
    match (panes.next(), panes.next()) {
        (Some(pane), None) if pane.pane_id == starter.pane_id => Some(starter.pane_id),
        _ => None,
    }
}

#[allow(dead_code)]
fn _state_path() -> Result<PathBuf> {
    storage::snapshot_path()
}
