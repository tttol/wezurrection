# wezurrection

`wezurrection` saves the layout and working directories of local WezTerm sessions and recreates them after restarting WezTerm or macOS.

It restores fresh shells. Running programs, shell memory, and scrollback are intentionally outside the first version.

## Usage

```sh
cargo install --path .
wezurrection version
wezurrection save
wezurrection restore
wezurrection enable
wezurrection status
wezurrection disable
```

`enable` installs a per-user macOS LaunchAgent. The agent waits for WezTerm, restores the latest snapshot once per launch, closes WezTerm's initial empty window after a successful restore, and saves changes every minute.

The snapshot is stored at `~/wezurrection/snapshot.json`. Existing snapshots from `~/Library/Application Support/wezurrection/snapshot.json` are read as a one-time compatibility fallback and new saves use the new location.
