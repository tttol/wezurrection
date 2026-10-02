use crate::session::{validate_snapshot, Snapshot};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSnapshot {
    snapshot: Snapshot,
}

pub fn state_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine the home directory")?;
    Ok(home.join("wezurrection"))
}

pub fn snapshot_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("snapshot.json"))
}

pub fn read_snapshot() -> Result<Option<Snapshot>> {
    let path = snapshot_path()?;
    let path = if path.exists() {
        path
    } else if let Some(legacy_path) = legacy_snapshot_path() {
        if legacy_path.exists() {
            legacy_path
        } else {
            return Ok(None);
        }
    } else {
        return Ok(None);
    };
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read snapshot at {}", path.display()))?;
    let stored: StoredSnapshot = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse snapshot at {}", path.display()))?;
    validate_snapshot(&stored.snapshot)?;
    Ok(Some(stored.snapshot))
}

fn legacy_snapshot_path() -> Option<PathBuf> {
    dirs::data_dir().map(|data_dir| data_dir.join("wezurrection/snapshot.json"))
}

pub fn write_snapshot(snapshot: &Snapshot) -> Result<()> {
    validate_snapshot(snapshot)?;
    let directory = state_dir()?;
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    let path = directory.join("snapshot.json");
    let temporary_path = directory.join("snapshot.json.tmp");
    let backup_path = directory.join("snapshot.json.bak");
    let content = serde_json::to_vec_pretty(&StoredSnapshot {
        snapshot: snapshot.clone(),
    })?;

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary_path)
        .with_context(|| format!("failed to open {}", temporary_path.display()))?;
    file.write_all(&content)
        .with_context(|| format!("failed to write {}", temporary_path.display()))?;
    file.sync_all()?;
    drop(file);

    if path.exists() {
        fs::copy(&path, &backup_path)
            .with_context(|| format!("failed to create {}", backup_path.display()))?;
    }
    fs::rename(&temporary_path, &path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

pub fn snapshot_age(snapshot: &Snapshot) -> String {
    let now = unix_time();
    let age = now.saturating_sub(snapshot.saved_at);
    match age {
        0..=59 => format!("{age}s ago"),
        60..=3599 => format!("{}m ago", age / 60),
        _ => format!("{}h ago", age / 3600),
    }
}

pub fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{PaneSnapshot, Snapshot, TabSnapshot, WindowSnapshot, SNAPSHOT_VERSION};

    #[test]
    fn snapshot_age_is_human_readable() {
        let snapshot = Snapshot {
            version: SNAPSHOT_VERSION,
            saved_at: unix_time().saturating_sub(120),
            windows: vec![WindowSnapshot {
                workspace: "default".to_owned(),
                title: String::new(),
                tabs: vec![TabSnapshot {
                    title: String::new(),
                    panes: vec![PaneSnapshot {
                        cwd: None,
                        left_col: 0,
                        top_row: 0,
                        cols: 80,
                        rows: 24,
                        active: true,
                        zoomed: false,
                    }],
                }],
            }],
        };
        assert!(snapshot_age(&snapshot).ends_with("m ago"));
    }
}
