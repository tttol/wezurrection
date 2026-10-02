use anyhow::{anyhow, bail, Result};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use url::Url;

pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub version: u32,
    pub saved_at: u64,
    pub windows: Vec<WindowSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowSnapshot {
    pub workspace: String,
    pub title: String,
    pub tabs: Vec<TabSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TabSnapshot {
    pub title: String,
    pub panes: Vec<PaneSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaneSnapshot {
    pub cwd: Option<String>,
    pub left_col: u64,
    pub top_row: u64,
    pub cols: u64,
    pub rows: u64,
    pub active: bool,
    pub zoomed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawPane {
    pub window_id: u64,
    pub tab_id: u64,
    pub pane_id: u64,
    pub workspace: String,
    pub size: RawPaneSize,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub left_col: u64,
    #[serde(default)]
    pub top_row: u64,
    #[serde(default)]
    pub tab_title: String,
    #[serde(default)]
    pub window_title: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub is_zoomed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawPaneSize {
    pub rows: u64,
    pub cols: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Orientation {
    Right,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutNode {
    Leaf(PaneSnapshot),
    Split {
        orientation: Orientation,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
        second_percent: u8,
    },
}

pub fn snapshot_from_raw(
    raw: &[RawPane],
    excluded_windows: &HashSet<u64>,
    saved_at: u64,
) -> Result<Snapshot> {
    let mut windows: BTreeMap<u64, BTreeMap<u64, Vec<&RawPane>>> = BTreeMap::new();
    raw.iter()
        .filter(|pane| !excluded_windows.contains(&pane.window_id))
        .for_each(|pane| {
            windows
                .entry(pane.window_id)
                .or_default()
                .entry(pane.tab_id)
                .or_default()
                .push(pane);
        });

    if windows.is_empty() {
        bail!("WezTerm returned no panes to save");
    }

    let windows = windows
        .into_values()
        .map(|tabs| {
            let first = tabs
                .values()
                .next()
                .and_then(|panes| panes.first())
                .ok_or_else(|| anyhow!("encountered a window without panes"))?;
            let workspace = first.workspace.clone();
            let window_title = first.window_title.clone();
            let tabs = tabs
                .into_values()
                .map(|mut panes| {
                    panes.sort_by_key(|pane| pane.pane_id);
                    let first = panes
                        .first()
                        .ok_or_else(|| anyhow!("encountered an empty tab"))?;
                    let tab_title = first.tab_title.clone();
                    let panes = panes
                        .into_iter()
                        .map(|pane| PaneSnapshot {
                            cwd: parse_cwd(&pane.cwd),
                            left_col: pane.left_col,
                            top_row: pane.top_row,
                            cols: pane.size.cols,
                            rows: pane.size.rows,
                            active: pane.is_active,
                            zoomed: pane.is_zoomed,
                        })
                        .collect();
                    Ok(TabSnapshot {
                        title: tab_title,
                        panes,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(WindowSnapshot {
                workspace,
                title: window_title,
                tabs,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Snapshot {
        version: SNAPSHOT_VERSION,
        saved_at,
        windows,
    })
}

pub fn validate_snapshot(snapshot: &Snapshot) -> Result<()> {
    if snapshot.version != SNAPSHOT_VERSION {
        bail!(
            "unsupported snapshot version {}; expected {}",
            snapshot.version,
            SNAPSHOT_VERSION
        );
    }
    if snapshot.windows.is_empty() {
        bail!("snapshot contains no windows");
    }
    for window in &snapshot.windows {
        if window.tabs.is_empty() {
            bail!("snapshot contains a window without tabs");
        }
        for tab in &window.tabs {
            if tab.panes.is_empty() {
                bail!("snapshot contains a tab without panes");
            }
            for pane in &tab.panes {
                if pane.cols == 0 || pane.rows == 0 {
                    bail!("snapshot contains a pane with invalid dimensions");
                }
            }
        }
    }
    Ok(())
}

pub fn layout_tree(panes: &[PaneSnapshot]) -> Result<LayoutNode> {
    if panes.is_empty() {
        bail!("cannot build a layout tree from no panes");
    }
    if panes.len() == 1 {
        return Ok(LayoutNode::Leaf(panes[0].clone()));
    }

    let min_left = panes.iter().map(|pane| pane.left_col).min().unwrap_or(0);
    let min_top = panes.iter().map(|pane| pane.top_row).min().unwrap_or(0);
    let max_right = panes
        .iter()
        .map(|pane| pane.left_col.saturating_add(pane.cols))
        .max()
        .unwrap_or(min_left);
    let max_bottom = panes
        .iter()
        .map(|pane| pane.top_row.saturating_add(pane.rows))
        .max()
        .unwrap_or(min_top);

    let vertical = candidate_boundaries(panes, true, min_left, max_right);
    let horizontal = candidate_boundaries(panes, false, min_top, max_bottom);

    let candidate = vertical
        .into_iter()
        .map(|(boundary, first, second)| (Orientation::Right, boundary, first, second))
        .chain(
            horizontal
                .into_iter()
                .map(|(boundary, first, second)| (Orientation::Bottom, boundary, first, second)),
        )
        .min_by_key(|(_, _, first, second)| first.len().abs_diff(second.len()));

    let Some((orientation, boundary, first, second)) = candidate else {
        bail!("cannot derive a rectangular split layout");
    };

    let total = match orientation {
        Orientation::Right => max_right.saturating_sub(min_left),
        Orientation::Bottom => max_bottom.saturating_sub(min_top),
    };
    let second_extent = match orientation {
        Orientation::Right => max_right.saturating_sub(boundary),
        Orientation::Bottom => max_bottom.saturating_sub(boundary),
    };
    let second_percent = percentage(second_extent, total);

    Ok(LayoutNode::Split {
        orientation,
        first: Box::new(layout_tree(&first)?),
        second: Box::new(layout_tree(&second)?),
        second_percent,
    })
}

pub fn first_leaf(node: &LayoutNode) -> &PaneSnapshot {
    match node {
        LayoutNode::Leaf(pane) => pane,
        LayoutNode::Split { first, .. } => first_leaf(first),
    }
}

fn candidate_boundaries(
    panes: &[PaneSnapshot],
    vertical: bool,
    min: u64,
    max: u64,
) -> Vec<(u64, Vec<PaneSnapshot>, Vec<PaneSnapshot>)> {
    let boundaries: BTreeSet<u64> = panes
        .iter()
        .flat_map(|pane| {
            let start = if vertical {
                pane.left_col
            } else {
                pane.top_row
            };
            let extent = if vertical { pane.cols } else { pane.rows };
            [start, start.saturating_add(extent)]
        })
        .filter(|boundary| *boundary > min && *boundary < max)
        .collect();

    boundaries
        .into_iter()
        .filter_map(|boundary| {
            let mut first = Vec::new();
            let mut second = Vec::new();
            for pane in panes {
                let start = if vertical {
                    pane.left_col
                } else {
                    pane.top_row
                };
                let end = start.saturating_add(if vertical { pane.cols } else { pane.rows });
                if end <= boundary {
                    first.push(pane.clone());
                } else if start >= boundary {
                    second.push(pane.clone());
                } else {
                    return None;
                }
            }
            (!first.is_empty() && !second.is_empty()).then_some((boundary, first, second))
        })
        .collect()
}

fn percentage(part: u64, whole: u64) -> u8 {
    if whole == 0 {
        return 50;
    }
    let value = ((part.saturating_mul(100) + whole / 2) / whole).clamp(1, 99);
    value as u8
}

fn parse_cwd(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(value) {
        if url.scheme() == "file" {
            return Some(
                percent_decode_str(url.path())
                    .decode_utf8_lossy()
                    .into_owned(),
            );
        }
    }
    Some(value.to_owned())
}

pub fn usable_cwd(cwd: &Option<String>, home: &Path) -> String {
    cwd.as_deref()
        .filter(|path| Path::new(path).is_dir())
        .unwrap_or_else(|| home.to_str().unwrap_or("/"))
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u64, left: u64, top: u64, cols: u64, rows: u64) -> PaneSnapshot {
        PaneSnapshot {
            cwd: Some(format!("/tmp/{id}")),
            left_col: left,
            top_row: top,
            cols,
            rows,
            active: id == 1,
            zoomed: false,
        }
    }

    #[test]
    fn builds_a_vertical_split() {
        let tree = layout_tree(&[pane(1, 0, 0, 40, 24), pane(2, 40, 0, 40, 24)]).unwrap();
        assert!(matches!(
            tree,
            LayoutNode::Split {
                orientation: Orientation::Right,
                second_percent: 50,
                ..
            }
        ));
    }

    #[test]
    fn builds_nested_layout() {
        let tree = layout_tree(&[
            pane(1, 0, 0, 40, 12),
            pane(2, 0, 12, 40, 12),
            pane(3, 40, 0, 40, 24),
        ])
        .unwrap();
        assert!(matches!(tree, LayoutNode::Split { .. }));
        assert_eq!(first_leaf(&tree).cwd.as_deref(), Some("/tmp/1"));
    }

    #[test]
    fn rejects_empty_layout() {
        assert!(layout_tree(&[]).is_err());
    }

    #[test]
    fn decodes_file_urls() {
        let raw = vec![RawPane {
            window_id: 1,
            tab_id: 1,
            pane_id: 1,
            workspace: "default".to_owned(),
            size: RawPaneSize { rows: 24, cols: 80 },
            title: "shell".to_owned(),
            cwd: "file://mac.local/Users/test/My%20Project/".to_owned(),
            left_col: 0,
            top_row: 0,
            tab_title: "tab".to_owned(),
            window_title: "window".to_owned(),
            is_active: true,
            is_zoomed: false,
        }];
        let snapshot = snapshot_from_raw(&raw, &HashSet::new(), 1).unwrap();
        assert_eq!(
            snapshot.windows[0].tabs[0].panes[0].cwd.as_deref(),
            Some("/Users/test/My Project/"),
        );
    }
}
