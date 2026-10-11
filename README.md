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

After a macOS reboot, snapshots saved before the current boot are restored even if macOS reopens WezTerm before the agent starts. Starting the agent while a session saved during the current boot is running does not restore it again. If restoration fails, automatic saves pause until WezTerm restarts to preserve the saved session. Temporary capture failures are logged without stopping the agent.

The snapshot is stored at `~/wezurrection/snapshot.json`. Existing snapshots from `~/Library/Application Support/wezurrection/snapshot.json` are read as a one-time compatibility fallback and new saves use the new location.

### Nix-managed WezTerm configuration

If `~/.wezterm.lua` links into `/nix/store`, `enable` installs a regular local loader at `~/.config/wezterm/wezterm.lua` when that path is unused. WezTerm reads this higher-priority file before creating its first window. The loader waits up to 60 seconds for the Nix-backed configuration to become readable, then loads it and watches it for changes. This prevents starting with defaults while the Nix volume is still mounting; no manual configuration reload is needed before automatic session restoration.

The original Nix-managed file remains the source of your settings. Existing configurations at the loader path are preserved. `disable` removes the loader only if it is still unchanged. Run `wezurrection enable` after installing this fix to install the loader.
