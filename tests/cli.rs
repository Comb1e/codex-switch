use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;
use toml_edit::DocumentMut;

struct IsolatedCli {
    _temp: TempDir,
    root: PathBuf,
    codex_home: PathBuf,
}

impl IsolatedCli {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let codex_home = root.join("codex");
        Self {
            _temp: temp,
            root,
            codex_home,
        }
    }

    fn command(&self) -> Command {
        #[allow(deprecated)]
        let mut command = Command::cargo_bin("cs").unwrap();
        command.env("CODEX_HOME", &self.codex_home);
        command.env("CS_TEST_SKIP_RELAY_START", "1");
        #[cfg(windows)]
        command
            .env("LOCALAPPDATA", self.root.join("local"))
            .env("APPDATA", self.root.join("roaming"))
            .env("USERPROFILE", self.root.join("home"));
        #[cfg(not(windows))]
        command
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("HOME", self.root.join("home"));
        command
    }

    fn auth_file(&self) -> PathBuf {
        self.codex_home.join("auth.json")
    }

    fn config_file(&self) -> PathBuf {
        self.codex_home.join("config.toml")
    }

    fn store_file(&self) -> PathBuf {
        #[cfg(windows)]
        let root = self.root.join("local");
        #[cfg(not(windows))]
        let root = self.root.join("config");
        root.join("cs").join("profiles.json")
    }
}

#[test]
fn add_switch_list_status_current_and_external_edit() {
    let cli = IsolatedCli::new();
    let secret = "key-\"quoted\\value";

    cli.command()
        .arg("status")
        .assert()
        .success()
        .stdout("Current provider: unmanaged\n\nNo profiles saved.\n");
    assert!(!cli.config_file().exists());
    assert!(!cli.store_file().exists());

    cli.command()
        .args([
            "add",
            "--name",
            "work",
            "--baseurl",
            "https://proxy.example.test/v1",
            "--apikey",
            secret,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Saved profile 'work'"))
        .stdout(predicate::str::contains(secret).not())
        .stderr(predicate::str::contains(secret).not());

    cli.command()
        .args([
            "add",
            "--name",
            "personal",
            "--baseurl",
            "https://personal.example.test/v1",
            "--apikey",
            "personal-secret",
        ])
        .assert()
        .success();
    cli.command()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Current provider: unmanaged"))
        .stdout(predicate::str::contains("personal"))
        .stdout(predicate::str::contains("work"))
        .stdout(predicate::str::contains("yes").not());

    fs::create_dir_all(&cli.codex_home).unwrap();
    let original_auth = br#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"other":"preserved"}"#;
    fs::write(cli.auth_file(), original_auth).unwrap();
    fs::write(
        cli.config_file(),
        "model = \"gpt-test\" # preserve me\n[features]\nweb_search = true\n",
    )
    .unwrap();

    cli.command()
        .arg("work")
        .assert()
        .success()
        .stdout(predicate::str::contains("Activated profile 'work'"));

    let config: DocumentMut = fs::read_to_string(cli.config_file())
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        config["model_provider"].as_str(),
        Some("codex_switch_relay")
    );
    assert_eq!(
        config["model_providers"]["codex_switch_relay"]["base_url"].as_str(),
        Some("http://127.0.0.1:14556/")
    );
    assert_eq!(
        config["model_providers"]["codex_switch_relay"]["requires_openai_auth"].as_bool(),
        Some(true)
    );
    assert_eq!(config["disable_response_storage"].as_bool(), Some(true));
    assert_eq!(config["network_access"].as_str(), Some("enabled"));
    assert_eq!(
        config["windows_wsl_setup_acknowledged"].as_bool(),
        Some(true)
    );
    assert_eq!(config["model"].as_str(), Some("gpt-test"));
    assert_eq!(config["features"]["web_search"].as_bool(), Some(true));
    assert_eq!(config["features"]["goals"].as_bool(), Some(true));
    assert_eq!(
        config["features"]["remote_compaction_v2"].as_bool(),
        Some(true)
    );

    let auth: Value = serde_json::from_slice(&fs::read(cli.auth_file()).unwrap()).unwrap();
    assert_eq!(auth["other"], "preserved");
    assert_eq!(fs::read(cli.auth_file()).unwrap(), original_auth);

    cli.command()
        .arg("current")
        .assert()
        .success()
        .stdout("work\n");
    cli.command()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("work"))
        .stdout(predicate::str::contains("yes"))
        .stdout(predicate::str::contains(secret).not());

    let original_config = fs::read(cli.config_file()).unwrap();
    let original_store = fs::read(cli.store_file()).unwrap();
    cli.command()
        .arg("status")
        .assert()
        .success()
        .stdout(
            "Current provider: work\n\nNAME      ACTIVE  BASE URL\npersonal          https://personal.example.test/v1\nwork      yes     https://proxy.example.test/v1\n",
        )
        .stdout(predicate::str::contains(secret).not())
        .stdout(predicate::str::contains("personal-secret").not())
        .stderr(predicate::str::contains(secret).not());
    assert_eq!(fs::read(cli.config_file()).unwrap(), original_config);
    assert_eq!(fs::read(cli.store_file()).unwrap(), original_store);
    assert_eq!(fs::read(cli.auth_file()).unwrap(), original_auth);

    for config in ["model_provider = \"other\"\n", "not = [toml", ""] {
        fs::write(cli.config_file(), config).unwrap();
        cli.command()
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Current provider: unmanaged"))
            .stdout(predicate::str::contains("personal"))
            .stdout(predicate::str::contains("work"))
            .stdout(predicate::str::contains("yes").not());
    }
    fs::remove_file(cli.config_file()).unwrap();
    cli.command()
        .arg("current")
        .assert()
        .success()
        .stdout("unmanaged\n");
    cli.command()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Current provider: unmanaged"))
        .stdout(predicate::str::contains("yes").not());

    fs::write(cli.config_file(), original_config).unwrap();
    let mut store: Value = serde_json::from_slice(&original_store).unwrap();
    store["profiles"].as_object_mut().unwrap().remove("work");
    fs::write(cli.store_file(), serde_json::to_vec(&store).unwrap()).unwrap();
    cli.command()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Current provider: unmanaged"))
        .stdout(predicate::str::contains("personal"))
        .stdout(predicate::str::contains("work").not())
        .stdout(predicate::str::contains("yes").not());
}

#[test]
fn status_rejects_extra_arguments_and_reserved_profile_name() {
    let cli = IsolatedCli::new();
    cli.command()
        .args(["status", "extra"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
    cli.command()
        .args([
            "add",
            "--name",
            "status",
            "--baseurl",
            "https://example.test/v1",
            "--apikey",
            "secret",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("reserved"));
    cli.command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("status"));

    fs::write(cli.store_file(), "not JSON").unwrap();
    cli.command()
        .arg("status")
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("profile store is invalid JSON"));
}

#[test]
fn resumes_the_last_selected_profile_in_a_new_process() {
    let cli = IsolatedCli::new();
    cli.command()
        .assert()
        .failure()
        .stderr(predicate::str::contains("no previous profile"));
    for name in ["first", "last"] {
        cli.command()
            .args([
                "add",
                "--name",
                name,
                "--baseurl",
                "https://example.test/v1",
                "--apikey",
                "test-key",
            ])
            .assert()
            .success();
        cli.command().arg(name).assert().success();
    }
    fs::write(cli.config_file(), "model_provider = \"other\"\n").unwrap();
    cli.command()
        .assert()
        .success()
        .stdout(predicate::str::contains("Activated profile 'last'"));
    cli.command()
        .arg("current")
        .assert()
        .success()
        .stdout("last\n");
    cli.command()
        .args(["remove", "last", "--force"])
        .assert()
        .success();
    cli.command()
        .assert()
        .failure()
        .stderr(predicate::str::contains("no previous profile"));
}

#[test]
fn accepts_api_key_from_stdin_and_never_echoes_it() {
    let cli = IsolatedCli::new();
    let secret = "stdin-secret";
    cli.command()
        .args([
            "add",
            "--name",
            "stdin",
            "--baseurl",
            "http://127.0.0.1:8080/v1",
            "--apikey-stdin",
        ])
        .write_stdin(format!("{secret}\r\n"))
        .assert()
        .success()
        .stdout(predicate::str::contains(secret).not())
        .stderr(predicate::str::contains(secret).not());

    let store: Value = serde_json::from_slice(&fs::read(cli.store_file()).unwrap()).unwrap();
    assert_eq!(store["profiles"]["stdin"]["api_key"], secret);
}

#[test]
fn rejects_duplicate_and_active_remove_without_force() {
    let cli = IsolatedCli::new();
    cli.command()
        .args([
            "add",
            "--name",
            "work",
            "--baseurl",
            "https://example.test/v1",
            "--apikey",
            "secret",
        ])
        .assert()
        .success();
    cli.command()
        .args([
            "add",
            "--name",
            "work",
            "--baseurl",
            "https://example.test/v1",
            "--apikey",
            "secret",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    cli.command().arg("work").assert().success();
    cli.command()
        .args(["remove", "work"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("active"));
    cli.command()
        .args(["remove", "work", "--force"])
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn secret_files_are_owner_only_on_linux() {
    use std::os::unix::fs::PermissionsExt;

    let cli = IsolatedCli::new();
    cli.command()
        .args([
            "add",
            "--name",
            "work",
            "--baseurl",
            "https://example.test/v1",
            "--apikey",
            "secret",
        ])
        .assert()
        .success();
    cli.command().arg("work").assert().success();

    assert_eq!(
        fs::metadata(cli.store_file()).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
