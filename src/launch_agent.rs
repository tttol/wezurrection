use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const LABEL: &str = "com.wezurrection.agent";

/// Returns the current macOS boot time in Unix seconds.
/// Fails if sysctl is unavailable or returns an unexpected value.
pub fn boot_time() -> Result<u64> {
    let output = Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.boottime"])
        .output()
        .context("failed to read macOS boot time")?;
    if !output.status.success() {
        anyhow::bail!(
            "could not read macOS boot time: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_boot_time(&String::from_utf8_lossy(&output.stdout))
}

fn parse_boot_time(value: &str) -> Result<u64> {
    value
        .split_once("sec =")
        .and_then(|(_, rest)| rest.split_once(','))
        .map(|(seconds, _)| seconds.trim())
        .context("macOS boot time did not contain seconds")?
        .parse()
        .context("macOS boot time contained invalid seconds")
}

pub fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine the home directory")?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

pub fn enable(executable: &str) -> Result<()> {
    let home = dirs::home_dir().context("could not determine the home directory")?;
    crate::startup_config::install(&home)?;
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
    let home = dirs::home_dir().context("could not determine the home directory")?;
    crate::startup_config::remove(&home)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_macos_boot_time() {
        // Given
        let value = "{ sec = 1791497975, usec = 877166 } Fri Oct 9 07:19:35 2026\n";
        let expected = 1_791_497_975;

        // When
        let actual = parse_boot_time(value).unwrap();

        // Then
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_invalid_boot_time() {
        for value in ["", "{ usec = 123 }", "{ sec = invalid, usec = 123 }"] {
            // When
            let actual = parse_boot_time(value);

            // Then
            assert!(actual.is_err(), "accepted {value:?}");
        }
    }
}
