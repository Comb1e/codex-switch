use std::fs::{self, File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result, bail};
use fs2::FileExt;

use crate::atomic::{FileSensitivity, FileWriter, ensure_private_dir};
use crate::cli::Action;
use crate::codex;
use crate::paths::AppPaths;
use crate::profile::{Profile, ProfileStore, ProviderMode};
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
            Action::Gpt => self.activate(ActivationTarget::Gpt),
            Action::List => self.list(),
            Action::Status => self.status(),
            Action::Current => self.current(),
            Action::Remove { name, force } => self.remove(name, force),
            Action::Activate { name } => self.activate(ActivationTarget::Profile(name)),
            Action::Resume => {
                let store = ProfileStore::load(&self.paths.store_file)?;
                let target = match store.mode {
                    ProviderMode::Gpt => ActivationTarget::Gpt,
                    ProviderMode::Relay => ActivationTarget::Profile(store.active.context(
                        "no previous profile; run 'cs <name>' or 'cs gpt' once to choose a default",
                    )?),
                };
                self.activate(target)
            }
            Action::RelayInstall => {
                let store = ProfileStore::load(&self.paths.store_file)?;
                if store.mode == ProviderMode::Gpt {
                    self.relay.stop()?;
                } else if store.active.is_some() {
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
        let active = active.filter(|_| store.mode == ProviderMode::Relay);
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

    fn activate(&self, target: ActivationTarget) -> Result<String> {
        let mut machine = SwitchMachine::new();
        let mut store = ProfileStore::load(&self.paths.store_file)?;
        machine.transition(SwitchPhase::Loaded);
        let mode = match &target {
            ActivationTarget::Profile(name) => {
                validate_name(name)?;
                let profile = store
                    .profiles
                    .get(name)
                    .ok_or_else(|| anyhow::anyhow!("profile '{name}' does not exist"))?;
                validate_base_url(&profile.base_url)?;
                validate_api_key(&profile.api_key)?;
                ProviderMode::Relay
            }
            ActivationTarget::Gpt => ProviderMode::Gpt,
        };
        machine.transition(SwitchPhase::Validated);

        ensure_private_dir(&self.paths.codex_home)?;
        let original_config = read_optional(&self.paths.config_file)?;
        let original_store = read_optional(&self.paths.store_file)?;
        let prepared_config = codex::prepare(original_config.as_deref(), mode)?;
        store.mode = mode;
        store.active = match &target {
            ActivationTarget::Profile(name) => Some(name.clone()),
            ActivationTarget::Gpt => None,
        };
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
            if !codex::file_matches(&self.paths.config_file, mode)? {
                bail!("Codex config did not match the selected provider mode after writing");
            }
            machine.transition(SwitchPhase::Verified);

            machine.transition(SwitchPhase::UpdatingRelay);
            match mode {
                ProviderMode::Relay => self.relay.restart(&self.paths)?,
                ProviderMode::Gpt => self.relay.stop()?,
            }
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

        Ok(match target {
            ActivationTarget::Profile(name) => format!(
                "Activated profile '{name}'. Codex auth.json was left unchanged; restart Codex to use the relay."
            ),
            ActivationTarget::Gpt => "Activated gpt mode (official OpenAI provider, no switch relay). Codex auth.json was left unchanged; restart Codex to connect directly.".into(),
        })
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
        if !codex::file_matches(&self.paths.config_file, store.mode)? {
            return Ok(None);
        }
        if store.mode == ProviderMode::Gpt {
            return Ok(Some("gpt"));
        }
        let Some(name) = store.active.as_deref() else {
            return Ok(None);
        };
        if !store.profiles.contains_key(name) {
            return Ok(None);
        }
        Ok(Some(name))
    }
}

enum ActivationTarget {
    Profile(String),
    Gpt,
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
    UpdatingRelay,
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
    use crate::profile::{ProfileStore, ProviderMode};
    use crate::relay::RelayManager;

    #[derive(Default)]
    struct TestRelay {
        restarts: AtomicUsize,
        stops: AtomicUsize,
    }

    impl RelayManager for TestRelay {
        fn restart(&self, _paths: &AppPaths) -> Result<()> {
            self.restarts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn stop(&self) -> Result<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
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
        let app = App::new(paths.clone(), RealFileWriter, TestRelay::default());

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
        let app = App::new(paths, RealFileWriter, TestRelay::default());
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
    fn gpt_uses_stop_and_resume_without_starting_a_relay() {
        let temp = tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path());
        let app = App::new(paths.clone(), RealFileWriter, TestRelay::default());
        app.execute(Action::Gpt).unwrap();
        app.execute(Action::Resume).unwrap();
        app.execute(Action::RelayInstall).unwrap();
        assert_eq!(app.relay.stops.load(Ordering::SeqCst), 3);
        assert_eq!(app.relay.restarts.load(Ordering::SeqCst), 0);
        assert_eq!(app.execute(Action::Current).unwrap(), "gpt");
        assert_eq!(
            ProfileStore::load(&paths.store_file).unwrap().mode,
            ProviderMode::Gpt
        );
        assert!(!paths.codex_home.join("auth.json").exists());

        app.execute(add_action()).unwrap();
        app.execute(Action::Activate {
            name: "work".into(),
        })
        .unwrap();
        assert_eq!(app.relay.restarts.load(Ordering::SeqCst), 1);
        assert_eq!(app.execute(Action::Current).unwrap(), "work");
    }

    #[test]
    fn rolls_back_write_failures_and_verification_corruption_in_either_mode() {
        for gpt in [false, true] {
            for fault in [
                Fault::FailBefore(1),
                Fault::FailBefore(2),
                Fault::Corrupt(2),
            ] {
                let temp = tempdir().unwrap();
                let paths = AppPaths::isolated(temp.path());
                let setup = App::new(paths.clone(), RealFileWriter, TestRelay::default());
                setup.execute(add_action()).unwrap();
                setup.execute(activation(!gpt)).unwrap();
                let auth_file = paths.codex_home.join("auth.json");
                fs::write(&auth_file, b"{\"original\":true}\n").unwrap();
                let original_config = fs::read(&paths.config_file).unwrap();
                let original_store = fs::read(&paths.store_file).unwrap();
                let app = App::new(paths.clone(), FaultWriter::new(fault), TestRelay::default());
                assert!(app.execute(activation(gpt)).is_err());
                assert_eq!(fs::read(&paths.config_file).unwrap(), original_config);
                assert_eq!(fs::read(&paths.store_file).unwrap(), original_store);
                assert_eq!(fs::read(&auth_file).unwrap(), b"{\"original\":true}\n");
                assert_eq!(app.relay.restarts.load(Ordering::SeqCst), 0);
                assert_eq!(app.relay.stops.load(Ordering::SeqCst), 0);
            }
        }
    }

    #[test]
    fn rolls_back_when_relay_restart_or_stop_fails() {
        struct FailingRelay;
        impl RelayManager for FailingRelay {
            fn restart(&self, _paths: &AppPaths) -> Result<()> {
                bail!("injected restart failure")
            }
            fn stop(&self) -> Result<()> {
                bail!("injected stop failure")
            }
        }
        for gpt in [false, true] {
            let temp = tempdir().unwrap();
            let paths = AppPaths::isolated(temp.path());
            let setup = App::new(paths.clone(), RealFileWriter, TestRelay::default());
            setup.execute(add_action()).unwrap();
            setup.execute(activation(!gpt)).unwrap();
            let original_config = fs::read(&paths.config_file).unwrap();
            let original_store = fs::read(&paths.store_file).unwrap();
            let app = App::new(paths.clone(), RealFileWriter, FailingRelay);
            let error = app.execute(activation(gpt)).unwrap_err();
            assert!(error.to_string().contains("UpdatingRelay"));
            assert_eq!(fs::read(&paths.config_file).unwrap(), original_config);
            assert_eq!(fs::read(&paths.store_file).unwrap(), original_store);
        }
    }

    fn activation(gpt: bool) -> Action {
        if gpt {
            Action::Gpt
        } else {
            Action::Activate {
                name: "work".into(),
            }
        }
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
        fn new(fault: Fault) -> Self {
            Self {
                writes: AtomicUsize::new(0),
                fault,
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
