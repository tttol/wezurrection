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
    Version,
    Enable,
    Disable,
    Status,
    Save,
    Restore,
    Watch,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
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
    // macOS can reopen WezTerm before launchd starts this watcher.
    let mut observed_absence = needs_startup_restore(
        storage::read_snapshot()?
            .as_ref()
            .map(|snapshot| snapshot.saved_at),
        launch_agent::boot_time()?,
    );
    let mut restored = false;
    let mut restore_failed = false;
    let mut ignored_windows = HashSet::new();
    let mut last_fingerprint = None;
    let mut last_save = Instant::now() - SAVE_INTERVAL;

    loop {
        match terminal.list() {
            Ok(panes) if !panes.is_empty() => {
                if !was_running {
                    restored = !observed_absence;
                    restore_failed = false;
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
                            Err(error) => {
                                restore_failed = true;
                                eprintln!("wezurrection: restore failed; automatic saves paused until WezTerm restarts: {error:#}");
                            }
                        }
                    }
                    restored = true;
                }

                if !restore_failed && last_save.elapsed() >= SAVE_INTERVAL {
                    match capture(&terminal, &ignored_windows) {
                        Ok(snapshot) => {
                            let fingerprint = serde_json::to_string(&snapshot.windows)?;
                            if last_fingerprint.as_deref() != Some(fingerprint.as_str()) {
                                storage::write_snapshot(&snapshot)?;
                                last_fingerprint = Some(fingerprint);
                            }
                        }
                        Err(error) => {
                            eprintln!("wezurrection: could not capture session: {error:#}");
                        }
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

fn needs_startup_restore(saved_at: Option<u64>, boot_time: u64) -> bool {
    saved_at.is_some_and(|saved_at| saved_at < boot_time)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_restore_depends_on_snapshot_boot() {
        // Given: an old snapshot must restore even when WezTerm is already running.
        let boot_time = 200;
        for (saved_at, expected) in [
            (Some(100), true),
            (Some(200), false),
            (Some(300), false),
            (None, false),
        ] {
            // When
            let actual = needs_startup_restore(saved_at, boot_time);

            // Then
            assert_eq!(actual, expected, "snapshot saved at {saved_at:?}");
        }
    }
}
