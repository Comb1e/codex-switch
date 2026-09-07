use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use directories::BaseDirs;

#[derive(Clone)]
pub struct AppPaths {
    pub store_file: PathBuf,
    pub lock_file: PathBuf,
    pub codex_home: PathBuf,
    pub config_file: PathBuf,
    pub relay_stdout_file: PathBuf,
    pub relay_stderr_file: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        #[cfg(windows)]
        let config_root = nonempty_env("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| BaseDirs::new().map(|dirs| dirs.data_local_dir().to_path_buf()))
            .context("could not determine the local application-data directory")?;

        #[cfg(not(windows))]
        let config_root = nonempty_env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| BaseDirs::new().map(|dirs| dirs.config_dir().to_path_buf()))
            .context("could not determine the user configuration directory")?;

        let codex_home = nonempty_env("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| BaseDirs::new().map(|dirs| dirs.home_dir().join(".codex")))
            .context("could not determine CODEX_HOME or the user home directory")?;
        if codex_home.as_os_str().is_empty() {
            bail!("CODEX_HOME must not be empty");
        }

        let store_dir = config_root.join("cs");
        Ok(Self {
            store_file: store_dir.join("profiles.json"),
            lock_file: store_dir.join(".lock"),
            relay_stdout_file: store_dir.join("relay.stdout.log"),
            relay_stderr_file: store_dir.join("relay.stderr.log"),
            config_file: codex_home.join("config.toml"),
            codex_home,
        })
    }

    #[cfg(test)]
    pub fn isolated(root: &std::path::Path) -> Self {
        let store_dir = root.join("store");
        let codex_home = root.join("codex");
        Self {
            store_file: store_dir.join("profiles.json"),
            lock_file: store_dir.join(".lock"),
            relay_stdout_file: store_dir.join("relay.stdout.log"),
            relay_stderr_file: store_dir.join("relay.stderr.log"),
            config_file: codex_home.join("config.toml"),
            codex_home,
        }
    }
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
