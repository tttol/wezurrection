use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

const LOADER: &str = include_str!("startup_config.lua");

/// Installs a local loader for a Nix-backed home configuration, without replacing
/// an existing higher-priority configuration. Errors on filesystem failures.
pub(crate) fn install(home: &Path) -> Result<()> {
    let source = home.join(".wezterm.lua");
    let target = match fs::read_link(&source) {
        Ok(target) => target,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::InvalidInput) => {
            return Ok(());
        }
        Err(error) => return Err(error).context("could not inspect WezTerm configuration"),
    };
    if !target.starts_with("/nix/store") {
        return Ok(());
    }

    let directory = home.join(".config/wezterm");
    fs::create_dir_all(&directory).context("could not create WezTerm configuration directory")?;
    let path = directory.join("wezterm.lua");
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error).context("could not create WezTerm startup loader"),
    };
    file.write_all(LOADER.as_bytes())
        .context("could not write WezTerm startup loader")?;
    file.sync_all()
        .context("could not sync WezTerm startup loader")?;
    Ok(())
}

/// Removes only the unchanged loader owned by wezurrection.
pub(crate) fn remove(home: &Path) -> Result<()> {
    let path = home.join(".config/wezterm/wezterm.lua");
    match fs::read_to_string(&path) {
        Ok(content) if content == LOADER => {
            fs::remove_file(path).context("could not remove WezTerm startup loader")?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("could not read WezTerm startup loader"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestHome(std::path::PathBuf);

    impl TestHome {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "wezurrection-config-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn installs_local_loader_even_when_nix_target_is_missing() {
        // Given
        let home = TestHome::new();
        symlink(
            "/nix/store/unmounted/wezterm.lua",
            home.0.join(".wezterm.lua"),
        )
        .unwrap();
        let expected = LOADER;

        // When
        install(&home.0).unwrap();

        // Then
        let path = home.0.join(".config/wezterm/wezterm.lua");
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert!(!fs::symlink_metadata(path).unwrap().file_type().is_symlink());
        assert_eq!(
            fs::read_link(home.0.join(".wezterm.lua")).unwrap(),
            Path::new("/nix/store/unmounted/wezterm.lua")
        );
    }

    #[test]
    fn preserves_existing_configuration_and_symlinks() {
        for linked in [false, true] {
            // Given
            let home = TestHome::new();
            symlink(
                "/nix/store/unmounted/wezterm.lua",
                home.0.join(".wezterm.lua"),
            )
            .unwrap();
            let directory = home.0.join(".config/wezterm");
            fs::create_dir_all(&directory).unwrap();
            let existing = directory.join("wezterm.lua");
            let expected = "return { font_size = 18 }";
            fs::write(home.0.join("custom.lua"), expected).unwrap();
            match linked {
                true => symlink(home.0.join("custom.lua"), &existing).unwrap(),
                false => fs::write(&existing, expected).unwrap(),
            }

            // When
            install(&home.0).unwrap();

            // Then
            assert_eq!(fs::read_to_string(&existing).unwrap(), expected);
            assert_eq!(
                fs::symlink_metadata(existing)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                linked
            );
        }
    }

    #[test]
    fn leaves_regular_home_configuration_unchanged() {
        // Given
        let home = TestHome::new();
        fs::write(home.0.join(".wezterm.lua"), "return {}").unwrap();

        // When
        install(&home.0).unwrap();

        // Then
        assert!(!home.0.join(".config/wezterm/wezterm.lua").exists());
    }

    #[test]
    fn removes_only_the_unchanged_managed_loader() {
        for (content, remains) in [(LOADER, false), ("return {}", true)] {
            // Given
            let home = TestHome::new();
            let directory = home.0.join(".config/wezterm");
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join("wezterm.lua");
            fs::write(&path, content).unwrap();

            // When
            remove(&home.0).unwrap();

            // Then
            assert_eq!(path.exists(), remains);
        }
    }
}
