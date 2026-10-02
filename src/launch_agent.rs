use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const LABEL: &str = "com.wezurrection.agent";

pub fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine the home directory")?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

pub fn enable(executable: &str) -> Result<()> {
    let path = plist_path()?;
    let parent = path.parent().context("launch agent path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let log_dir = crate::storage::state_dir()?;
    fs::create_dir_all(&log_dir)?;
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array><string>{}</string><string>watch</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",
        xml_escape(executable),
        xml_escape(&log_dir.join("agent.log").display().to_string()),
        xml_escape(&log_dir.join("agent.error.log").display().to_string()),
    );
    fs::write(&path, plist).with_context(|| format!("failed to write {}", path.display()))?;
    let domain = format!("gui/{}", current_uid()?);
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, path.to_str().unwrap_or_default()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    run_launchctl(["bootstrap", &domain, path.to_str().unwrap_or_default()])?;
    Ok(())
}

pub fn disable() -> Result<()> {
    let path = plist_path()?;
    let domain = format!("gui/{}", current_uid()?);
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, path.to_str().unwrap_or_default()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

pub fn is_running() -> Result<bool> {
    let domain = format!("gui/{}/{}", current_uid()?, LABEL);
    Ok(Command::new("launchctl")
        .args(["print", &domain])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success())
}

fn run_launchctl<const N: usize>(args: [&str; N]) -> Result<()> {
    let output = Command::new("launchctl")
        .args(args)
        .output()
        .context("failed to run launchctl")?;
    if !output.status.success() {
        anyhow::bail!(
            "launchctl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn current_uid() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to determine user id")?;
    if !output.status.success() {
        anyhow::bail!("id -u failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
