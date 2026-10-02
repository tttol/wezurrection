use crate::session::{
    first_leaf, layout_tree, usable_cwd, LayoutNode, Orientation, RawPane, Snapshot,
};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

const WEZTERM: &str = "wezterm";
const WEZTERM_PATHS: &[&str] = &[
    "/opt/homebrew/bin/wezterm",
    "/usr/local/bin/wezterm",
    "/Applications/WezTerm.app/Contents/MacOS/wezterm",
];

#[derive(Debug, Clone)]
pub struct WezTerm {
    executable: String,
}

impl Default for WezTerm {
    fn default() -> Self {
        Self {
            executable: WEZTERM_PATHS
                .iter()
                .find(|path| Path::new(path).is_file())
                .copied()
                .unwrap_or(WEZTERM)
                .to_owned(),
        }
    }
}

impl WezTerm {
    pub fn list(&self) -> Result<Vec<RawPane>> {
        let output = Command::new(&self.executable)
            .args(["cli", "--no-auto-start", "list", "--format", "json"])
            .output()
            .with_context(|| format!("failed to run {}", self.executable))?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            bail!(
                "WezTerm is not available: {}",
                if message.is_empty() {
                    "the CLI returned an error"
                } else {
                    &message
                }
            );
        }
        serde_json::from_slice(&output.stdout).context("failed to parse WezTerm pane list")
    }

    pub fn spawn_window(&self, workspace: &str, cwd: &str) -> Result<u64> {
        self.spawn([
            "spawn".to_owned(),
            "--new-window".to_owned(),
            "--workspace".to_owned(),
            workspace.to_owned(),
            "--cwd".to_owned(),
            cwd.to_owned(),
        ])
    }

    pub fn spawn_tab(&self, window_id: u64, cwd: &str) -> Result<u64> {
        self.spawn([
            "spawn".to_owned(),
            "--window-id".to_owned(),
            window_id.to_string(),
            "--cwd".to_owned(),
            cwd.to_owned(),
        ])
    }

    pub fn kill_pane(&self, pane_id: u64) -> Result<()> {
        self.run(["kill-pane", "--pane-id", &pane_id.to_string()])
            .map(|_| ())
    }

    pub fn split(&self, pane_id: u64, node: &LayoutNode, home: &Path) -> Result<u64> {
        let LayoutNode::Split {
            orientation,
            second,
            second_percent,
            ..
        } = node
        else {
            bail!("cannot split a leaf pane");
        };
        let direction = match orientation {
            Orientation::Right => "--right",
            Orientation::Bottom => "--bottom",
        };
        let cwd = usable_cwd(
            &Some(first_leaf(second).cwd.clone().unwrap_or_default()),
            home,
        );
        self.spawn([
            "split-pane".to_owned(),
            "--pane-id".to_owned(),
            pane_id.to_string(),
            direction.to_owned(),
            "--percent".to_owned(),
            second_percent.to_string(),
            "--cwd".to_owned(),
            cwd,
        ])
    }

    pub fn set_tab_title(&self, pane_id: u64, title: &str) -> Result<()> {
        self.run(["set-tab-title", "--pane-id", &pane_id.to_string(), title])
            .map(|_| ())
    }

    pub fn set_window_title(&self, window_id: u64, title: &str) -> Result<()> {
        self.run([
            "set-window-title",
            "--window-id",
            &window_id.to_string(),
            title,
        ])
        .map(|_| ())
    }

    pub fn activate_pane(&self, pane_id: u64) -> Result<()> {
        self.run(["activate-pane", "--pane-id", &pane_id.to_string()])
            .map(|_| ())
    }

    pub fn restore(&self, snapshot: &Snapshot, home: &Path) -> Result<()> {
        for window in &snapshot.windows {
            let mut actual_window = None;
            let mut active_pane = None;
            for (tab_index, tab) in window.tabs.iter().enumerate() {
                let tree = layout_tree(&tab.panes).with_context(|| {
                    format!("failed to derive layout for tab {}", tab_index + 1)
                })?;
                let root = first_leaf(&tree);
                let root_cwd = usable_cwd(&root.cwd, home);
                let pane_id = if let Some(window_id) = actual_window {
                    self.spawn_tab(window_id, &root_cwd)?
                } else {
                    let pane_id = self.spawn_window(&window.workspace, &root_cwd)?;
                    let discovered = self
                        .list()?
                        .into_iter()
                        .find(|pane| pane.pane_id == pane_id)
                        .ok_or_else(|| {
                            anyhow!("could not find newly spawned WezTerm pane {pane_id}")
                        })?;
                    actual_window = Some(discovered.window_id);
                    pane_id
                };

                self.set_tab_title(pane_id, &tab.title)?;
                let restored_active = self.restore_node(pane_id, &tree, home)?;
                active_pane = restored_active.or(active_pane);
                if tab.panes.iter().any(|pane| pane.active) {
                    active_pane = restored_active.or(active_pane);
                }
            }
            if let Some(window_id) = actual_window {
                self.set_window_title(window_id, &window.title)?;
            }
            if let Some(pane_id) = active_pane {
                self.activate_pane(pane_id)?;
            }
        }
        Ok(())
    }

    fn restore_node(
        &self,
        target_pane: u64,
        node: &LayoutNode,
        home: &Path,
    ) -> Result<Option<u64>> {
        match node {
            LayoutNode::Leaf(pane) => Ok(pane.active.then_some(target_pane)),
            LayoutNode::Split { first, second, .. } => {
                let new_pane = self.split(target_pane, node, home)?;
                let first_active = self.restore_node(target_pane, first, home)?;
                let second_active = self.restore_node(new_pane, second, home)?;
                Ok(second_active.or(first_active))
            }
        }
    }

    fn spawn<I>(&self, args: I) -> Result<u64>
    where
        I: IntoIterator<Item = String>,
    {
        let output = self.run(args)?;
        let pane_id = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .context("WezTerm did not return a pane id")?;
        Ok(pane_id)
    }

    fn run<I, S>(&self, args: I) -> Result<std::process::Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = Command::new(&self.executable)
            .arg("cli")
            .arg("--no-auto-start")
            .args(args)
            .output()
            .with_context(|| format!("failed to run {}", self.executable))?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            bail!(
                "WezTerm command failed: {}",
                if message.is_empty() {
                    "unknown error"
                } else {
                    &message
                }
            );
        }
        Ok(output)
    }
}

pub fn pane_window_ids(panes: &[RawPane]) -> HashMap<u64, Vec<u64>> {
    panes.iter().fold(HashMap::new(), |mut windows, pane| {
        windows
            .entry(pane.window_id)
            .or_default()
            .push(pane.pane_id);
        windows
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_panes_by_window() {
        let panes = vec![
            RawPane {
                window_id: 1,
                tab_id: 1,
                pane_id: 7,
                workspace: "default".to_owned(),
                size: crate::session::RawPaneSize { rows: 24, cols: 80 },
                title: String::new(),
                cwd: String::new(),
                left_col: 0,
                top_row: 0,
                tab_title: String::new(),
                window_title: String::new(),
                is_active: false,
                is_zoomed: false,
            },
            RawPane {
                window_id: 2,
                pane_id: 8,
                ..RawPane {
                    window_id: 1,
                    tab_id: 1,
                    pane_id: 8,
                    workspace: "default".to_owned(),
                    size: crate::session::RawPaneSize { rows: 24, cols: 80 },
                    title: String::new(),
                    cwd: String::new(),
                    left_col: 0,
                    top_row: 0,
                    tab_title: String::new(),
                    window_title: String::new(),
                    is_active: false,
                    is_zoomed: false,
                }
            },
        ];
        let groups = pane_window_ids(&panes);
        assert_eq!(groups.get(&1), Some(&vec![7]));
        assert_eq!(groups.get(&2), Some(&vec![8]));
    }
}
