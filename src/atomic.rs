use std::fs::{self, File, Permissions};
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use atomicwrites::{AllowOverwrite, AtomicFile};

#[derive(Clone, Copy)]
pub enum FileSensitivity {
    Secret,
    Regular,
}

pub trait FileWriter {
    fn write(&self, path: &Path, bytes: &[u8], sensitivity: FileSensitivity) -> Result<()>;
    fn remove(&self, path: &Path) -> Result<()>;
}

#[derive(Clone, Copy)]
pub struct RealFileWriter;

impl FileWriter for RealFileWriter {
    fn write(&self, path: &Path, bytes: &[u8], sensitivity: FileSensitivity) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
        ensure_private_dir(parent)?;

        let prior_permissions = fs::metadata(path)
            .ok()
            .map(|metadata| metadata.permissions());
        AtomicFile::new(path, AllowOverwrite)
            .write(|file| {
                apply_permissions(file, sensitivity, prior_permissions.as_ref())?;
                file.write_all(bytes)?;
                file.sync_all()
            })
            .map_err(|error| {
                anyhow::anyhow!("atomic write failed for {}: {error}", path.display())
            })?;

        sync_parent(parent)?;
        Ok(())
    }

    fn remove(&self, path: &Path) -> Result<()> {
        match fs::remove_file(path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    sync_parent(parent)?;
                }
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("could not remove {}", path.display()))
            }
        }
    }
}

pub fn ensure_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("could not create directory {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, Permissions::from_mode(0o700))
            .with_context(|| format!("could not secure directory {}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn apply_permissions(
    file: &File,
    sensitivity: FileSensitivity,
    prior: Option<&Permissions>,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = match sensitivity {
        FileSensitivity::Secret => Permissions::from_mode(0o600),
        FileSensitivity::Regular => prior
            .cloned()
            .unwrap_or_else(|| Permissions::from_mode(0o600)),
    };
    file.set_permissions(permissions)
}

#[cfg(not(unix))]
fn apply_permissions(
    _file: &File,
    _sensitivity: FileSensitivity,
    _prior: Option<&Permissions>,
) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<()> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("could not sync directory {}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<()> {
    Ok(())
}
