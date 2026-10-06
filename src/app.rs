use std::fs::{self, File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result, bail};
use fs2::FileExt;

use crate::atomic::{FileSensitivity, FileWriter, ensure_private_dir};
use crate::cli::Action;
use crate::codex;
use crate::paths::AppPaths;
use crate::profile::{Profile, ProfileStore};
use crate::relay::{self, RelayManager};
use crate::validation::{validate_api_key, validate_base_url, validate_name};

pub struct App<W, R> {
    paths: AppPaths,
    writer: W,
    relay: R,
}

impl<W: FileWriter, R: RelayManager> App<W, R> {
    pub fn new(paths: AppPaths, writer: W, relay: R) -> Self {
        Self {
            paths,
            writer,
            relay,
        }
    }

    pub fn execute(&self, action: Action) -> Result<String> {
        if let Action::Relay { data_dir } = action {
            let paths = match data_dir {
                Some(directory) => self.paths.clone().with_data_dir(directory),
                None => self.paths.clone(),
            };
            relay::serve(paths)?;
            return Ok(String::new());
        }
        let _lock = LockGuard::acquire(&self.paths.lock_file)?;
        match action {
            Action::Add {
                name,
                base_url,
                api_key,
                force,
            } => self.add(name, base_url, api_key, force),
            Action::List => self.list(),
            Action::Status => self.status(),
            Action::Current => self.current(),
            Action::Remove { name, force } => self.remove(name, force),
            Action::Activate { name } => self.activate(name),
            Action::Resume => {
                let name = ProfileStore::load(&self.paths.store_file)?
                    .active
                    .context("no previous profile; run 'cs <name>' once to choose a default")?;
                self.activate(name)
            }
            Action::RelayInstall => {
                if ProfileStore::load(&self.paths.store_file)?.active.is_some() {
                    self.relay.restart(&self.paths)?;
                }
                Ok(String::new())
            }
            Action::Relay { .. } => unreachable!("relay action was handled before locking"),
        }
    }

    fn add(&self, name: String, base_url: String, api_key: String, force: bool) -> Result<String> {
        validate_name(&name)?;
        validate_base_url(&base_url)?;
        validate_api_key(&api_key)?;

        let mut store = ProfileStore::load(&self.paths.store_file)?;
        if store.profiles.contains_key(&name) && !force {
            bail!("profile '{name}' already exists; use --force to replace it");
        }
        if store.active.as_deref() == Some(name.as_str()) {
            store.active = None;
        }
        store
            .profiles
            .insert(name.clone(), Profile { base_url, api_key });
        store.save(&self.paths.store_file, &self.writer)?;

        Ok(format!(
            "Saved profile '{name}'. Run 'cs {name}' to activate it."
        ))
    }

    fn list(&self) -> Result<String> {
        let store = ProfileStore::load(&self.paths.store_file)?;
        let active = self.verified_active(&store)?;
        Ok(Self::format_profiles(&store, active))
    }

    fn status(&self) -> Result<String> {
        let store = ProfileStore::load(&self.paths.store_file)?;
        let active = self.verified_active(&store)?;
        Ok(format!(
            "Current provider: {}\n\n{}",
            active.unwrap_or("unmanaged"),
            Self::format_profiles(&store, active)
        ))
    }

    fn format_profiles(store: &ProfileStore, active: Option<&str>) -> String {
        if store.profiles.is_empty() {
            return "No profiles saved.".into();
        }
        let width = store
            .profiles
            .keys()
            .map(String::len)
            .max()
            .unwrap_or(4)
            .max(4);
        let mut lines = vec![format!(
            "{:<width$}  {:<6}  BASE URL",
            "NAME",
            "ACTIVE",
            width = width
        )];
        for (name, profile) in &store.profiles {
            let marker = if active == Some(name.as_str()) {
                "yes"
            } else {
                ""
            };
            lines.push(format!(
                "{:<width$}  {:<6}  {}",
                name,
                marker,
                profile.base_url,
                width = width
            ));
        }
        lines.join("\n")
    }

    fn current(&self) -> Result<String> {
        let store = ProfileStore::load(&self.paths.store_file)?;
        Ok(self.verified_active(&store)?.unwrap_or("unmanaged").into())
    }

    fn remove(&self, name: String, force: bool) -> Result<String> {
        validate_name(&name)?;
        let mut store = ProfileStore::load(&self.paths.store_file)?;
        if !store.profiles.contains_key(&name) {
            bail!("profile '{name}' does not exist");
        }
        let is_active = store.active.as_deref() == Some(name.as_str());
        if is_active && !force {
            bail!("profile '{name}' is active; use --force to remove the saved profile");
        }
        store.profiles.remove(&name);
        if is_active {
            store.active = None;
        }
        store.save(&self.paths.store_file, &self.writer)?;
        Ok(format!("Removed profile '{name}'."))
    }

    fn activate(&self, name: String) -> Result<String> {
        let mut machine = SwitchMachine::new();
        validate_name(&name)?;

        let mut store = ProfileStore::load(&self.paths.store_file)?;
        machine.transition(SwitchPhase::Loaded);
        let profile = store
            .profiles
            .get(&name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("profile '{name}' does not exist"))?;
        validate_base_url(&profile.base_url)?;
        validate_api_key(&profile.api_key)?;
        machine.transition(SwitchPhase::Validated);

        ensure_private_dir(&self.paths.codex_home)?;
        let original_config = read_optional(&self.paths.config_file)?;
        let original_store = read_optional(&self.paths.store_file)?;
        let prepared_config = codex::prepare(original_config.as_deref())?;
        store.active = Some(name.clone());
        let active_store = store.encode()?;
        machine.transition(SwitchPhase::Staged);

        let result = (|| -> Result<()> {
            machine.transition(SwitchPhase::RecordingActive);
            self.writer.write(
                &self.paths.store_file,
                &active_store,
                FileSensitivity::Secret,
            )?;
            machine.transition(SwitchPhase::ActiveRecorded);

            machine.transition(SwitchPhase::WritingConfig);
            self.writer.write(
                &self.paths.config_file,
                &prepared_config,
                FileSensitivity::Regular,
            )?;
            machine.transition(SwitchPhase::ConfigWritten);

            machine.transition(SwitchPhase::Verifying);
            if !codex::file_matches(&self.paths.config_file)? {
                bail!("Codex config did not match the relay configuration after writing");
            }
            machine.transition(SwitchPhase::Verified);

            machine.transition(SwitchPhase::RestartingRelay);
            self.relay.restart(&self.paths)?;
            machine.transition(SwitchPhase::RelayReady);
            machine.transition(SwitchPhase::Done);
            Ok(())
        })();

        if let Err(error) = result {
            if machine.phase() >= SwitchPhase::RecordingActive {
                let rollback_errors =
                    self.rollback(original_config.as_deref(), original_store.as_deref());
                if !rollback_errors.is_empty() {
                    bail!(
                        "switch failed during {:?}: {error:#}; rollback also failed: {}",
                        machine.phase(),
                        rollback_errors.join("; ")
                    );
                }
            }
            return Err(error)
                .with_context(|| format!("switch failed during {:?}", machine.phase()));
        }

        Ok(format!(
            "Activated profile '{name}'. Codex auth.json was left unchanged; restart Codex to use the relay."
        ))
    }

    fn rollback(&self, config: Option<&[u8]>, store: Option<&[u8]>) -> Vec<String> {
        let mut errors = Vec::new();
        for (path, bytes, sensitivity) in [
            (
                self.paths.store_file.as_path(),
                store,
                FileSensitivity::Secret,
            ),
            (
                self.paths.config_file.as_path(),
                config,
                FileSensitivity::Regular,
            ),
        ] {
            let result = match bytes {
                Some(bytes) => self.writer.write(path, bytes, sensitivity),
                None => self.writer.remove(path),
            };
            if let Err(error) = result {
                errors.push(format!("{}: {error:#}", path.display()));
            }
        }
        errors
    }

    fn verified_active<'a>(&self, store: &'a ProfileStore) -> Result<Option<&'a str>> {
        let Some(name) = store.active.as_deref() else {
            return Ok(None);
        };
        if !store.profiles.contains_key(name) {
            return Ok(None);
        }
        if codex::file_matches(&self.paths.config_file)? {
            Ok(Some(name))
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SwitchPhase {
    Locked,
    Loaded,
    Validated,
    Staged,
    RecordingActive,
    ActiveRecorded,
    WritingConfig,
    ConfigWritten,
    Verifying,
    Verified,
    RestartingRelay,
    RelayReady,
    Done,
}

struct SwitchMachine {
    phase: SwitchPhase,
}

impl SwitchMachine {
    fn new() -> Self {
        Self {
            phase: SwitchPhase::Locked,
        }
    }

    fn transition(&mut self, next: SwitchPhase) {
        debug_assert!(next > self.phase, "switch phases must move forward");
        self.phase = next;
    }

    fn phase(&self) -> SwitchPhase {
        self.phase
    }
}

pub(crate) struct LockGuard {
    file: File,
}

impl LockGuard {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let file = Self::open(path)?;
        file.lock_exclusive()
            .with_context(|| format!("could not lock {}", path.display()))?;
        Ok(Self { file })
    }

    pub(crate) fn try_acquire(path: &Path) -> Result<Option<Self>> {
        let file = Self::open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(error) if error.kind() == fs2::lock_contended_error().kind() => Ok(None),
            Err(error) => Err(error).with_context(|| format!("could not lock {}", path.display())),
        }
    }

    fn open(path: &Path) -> Result<File> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("lock path has no parent: {}", path.display()))?;
        ensure_private_dir(parent)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("could not open lock file {}", path.display()))?;
        Ok(file)
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("could not read existing file {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::{Result, bail};
    use tempfile::tempdir;

    use super::App;
    use crate::atomic::{FileSensitivity, FileWriter, RealFileWriter};
    use crate::cli::Action;
    use crate::paths::AppPaths;
    use crate::profile::ProfileStore;
    use crate::relay::RelayManager;

    #[derive(Clone, Copy)]
    struct TestRelay;

    impl RelayManager for TestRelay {
        fn restart(&self, _paths: &AppPaths) -> Result<()> {
            Ok(())
        }
    }

    fn add_action() -> Action {
        Action::Add {
            name: "work".into(),
            base_url: "https://proxy.example.test/v1".into(),
            api_key: "secret-key".into(),
            force: false,
        }
    }

    #[test]
    fn supports_profile_lifecycle_and_detects_external_edits() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path());
        let app = App::new(paths.clone(), RealFileWriter, TestRelay);

        let added = app.execute(add_action()).unwrap();
        assert!(!added.contains("secret-key"));
        assert_eq!(app.execute(Action::Current).unwrap(), "unmanaged");
        app.execute(Action::Activate {
            name: "work".into(),
        })
        .unwrap();
        assert_eq!(app.execute(Action::Current).unwrap(), "work");
        let listed = app.execute(Action::List).unwrap();
        assert!(listed.contains("https://proxy.example.test/v1"));
        assert!(!listed.contains("secret-key"));

        fs::write(&paths.config_file, "model_provider = \"other\"\n").unwrap();
        assert_eq!(app.execute(Action::Current).unwrap(), "unmanaged");
    }

    #[test]
    fn duplicate_and_active_removal_require_force() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path());
        let app = App::new(paths, RealFileWriter, TestRelay);
        app.execute(add_action()).unwrap();
        assert!(app.execute(add_action()).is_err());
        app.execute(Action::Activate {
            name: "work".into(),
        })
        .unwrap();
        assert!(
            app.execute(Action::Remove {
                name: "work".into(),
                force: false
            })
            .is_err()
        );
        app.execute(Action::Remove {
            name: "work".into(),
            force: true,
        })
        .unwrap();
    }

    #[test]
    fn rolls_back_when_config_or_active_store_writes_fail() {
        for fail_on in 1..=2 {
            let temp = tempdir().unwrap();
            let paths = AppPaths::isolated(temp.path());
            let setup = App::new(paths.clone(), RealFileWriter, TestRelay);
            setup.execute(add_action()).unwrap();
            fs::create_dir_all(&paths.codex_home).unwrap();
            fs::write(&paths.config_file, b"model = \"original\"\n").unwrap();
            let auth_file = paths.codex_home.join("auth.json");
            fs::write(&auth_file, b"{\"original\":true}\n").unwrap();
            let original_store = fs::read(&paths.store_file).unwrap();

            let app = App::new(paths.clone(), FaultWriter::fail_before(fail_on), TestRelay);
            assert!(
                app.execute(Action::Activate {
                    name: "work".into()
                })
                .is_err()
            );
            assert_eq!(
                fs::read(&paths.config_file).unwrap(),
                b"model = \"original\"\n"
            );
            assert_eq!(fs::read(&auth_file).unwrap(), b"{\"original\":true}\n");
            assert_eq!(fs::read(&paths.store_file).unwrap(), original_store);
            assert!(
                ProfileStore::load(&paths.store_file)
                    .unwrap()
                    .active
                    .is_none()
            );
        }
    }

    #[test]
    fn rolls_back_when_verification_detects_corruption() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path());
        let setup = App::new(paths.clone(), RealFileWriter, TestRelay);
        setup.execute(add_action()).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        fs::write(&paths.config_file, b"# original\n").unwrap();
        let auth_file = paths.codex_home.join("auth.json");
        fs::write(&auth_file, b"{}\n").unwrap();

        let app = App::new(paths.clone(), FaultWriter::corrupt(2), TestRelay);
        assert!(
            app.execute(Action::Activate {
                name: "work".into()
            })
            .is_err()
        );
        assert_eq!(fs::read(&paths.config_file).unwrap(), b"# original\n");
        assert_eq!(fs::read(&auth_file).unwrap(), b"{}\n");
    }

    enum Fault {
        FailBefore(usize),
        Corrupt(usize),
    }

    struct FaultWriter {
        writes: AtomicUsize,
        fault: Fault,
    }

    impl FaultWriter {
        fn fail_before(write: usize) -> Self {
            Self {
                writes: AtomicUsize::new(0),
                fault: Fault::FailBefore(write),
            }
        }

        fn corrupt(write: usize) -> Self {
            Self {
                writes: AtomicUsize::new(0),
                fault: Fault::Corrupt(write),
            }
        }
    }

    impl FileWriter for FaultWriter {
        fn write(&self, path: &Path, bytes: &[u8], sensitivity: FileSensitivity) -> Result<()> {
            let write = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
            match self.fault {
                Fault::FailBefore(target) if write == target => bail!("injected write failure"),
                Fault::Corrupt(target) if write == target => {
                    RealFileWriter.write(path, b"corrupt", sensitivity)
                }
                _ => RealFileWriter.write(path, bytes, sensitivity),
            }
        }

        fn remove(&self, path: &Path) -> Result<()> {
            RealFileWriter.remove(path)
        }
    }
}
