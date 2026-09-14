#![cfg(debug_assertions)]

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde_json::{Value, json};
use tiny_http::{Response, Server};

struct BackgroundRelay {
    supervisor: Child,
    client: Client,
    base_url: String,
}

impl BackgroundRelay {
    fn health_pid(&self) -> Option<u64> {
        let response = self
            .client
            .get(format!("{}/__codex_switch_health", self.base_url))
            .send()
            .ok()?
            .text()
            .ok()?;
        let health: Value = serde_json::from_str(&response).ok()?;
        assert_eq!(health["service"], "codex-switch-relay");
        health["pid"].as_u64()
    }

    fn wait_ready(&mut self, previous: Option<u64>) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(
                self.supervisor.try_wait().unwrap().is_none(),
                "supervisor exited"
            );
            if let Some(pid) = self.health_pid()
                && Some(pid) != previous
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "relay did not recover");
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn shutdown_worker(&self) {
        self.client
            .post(format!("{}/__codex_switch_shutdown", self.base_url))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap();
    }
}

impl Drop for BackgroundRelay {
    fn drop(&mut self) {
        let _ = self.supervisor.kill();
        let _ = self.supervisor.wait();
        let _ = self
            .client
            .post(format!("{}/__codex_switch_shutdown", self.base_url))
            .send();
    }
}

fn launcher(data_dir: &Path, unrelated_root: &Path, port: u16) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cs-relay"));
    command
        .arg("--data-dir")
        .arg(data_dir)
        .env("CS_TEST_RELAY_PORT", port.to_string())
        .env("LOCALAPPDATA", unrelated_root)
        .env("XDG_CONFIG_HOME", unrelated_root)
        .env("CODEX_HOME", unrelated_root.join("codex"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[test]
fn login_restores_saved_profile_without_a_terminal_and_recovers_worker_exit() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("saved profiles with spaces");
    fs::create_dir_all(&data_dir).unwrap();
    let upstream = Server::http("127.0.0.1:0").unwrap();
    fs::write(data_dir.join("profiles.json"), serde_json::to_vec(&json!({
        "version": 1,
        "active": "last",
        "profiles": {
            "first": { "base_url": "http://127.0.0.1:1", "api_key": "wrong-key" },
            "last": { "base_url": format!("http://{}", upstream.server_addr()), "api_key": "last-key" }
        }
    })).unwrap()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let unrelated = temp.path().join("different login environment");
    let mut relay = BackgroundRelay {
        supervisor: launcher(&data_dir, &unrelated, port).spawn().unwrap(),
        client: Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
        base_url: format!("http://127.0.0.1:{port}"),
    };
    let original_pid = relay.wait_ready(None);

    // Repeated login/activation launches must not create competing supervisors.
    let mut duplicate = launcher(&data_dir, &unrelated, port).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = duplicate.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = duplicate.kill();
            let _ = duplicate.wait();
            panic!("duplicate supervisor did not exit");
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(relay.health_pid(), Some(original_pid));

    let (request_started_tx, request_started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let upstream_thread = thread::spawn(move || {
        for index in 0..2 {
            let request = upstream
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .unwrap();
            assert_eq!(
                request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("authorization"))
                    .unwrap()
                    .value
                    .as_str(),
                "Bearer last-key"
            );
            if index == 0 {
                request_started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            request
                .respond(Response::from_string("saved-profile-ok"))
                .unwrap();
        }
    });
    let forward = |relay: &BackgroundRelay| {
        assert_eq!(
            relay
                .client
                .post(format!("{}/responses", relay.base_url))
                .bearer_auth("incoming-test-token")
                .send()
                .unwrap()
                .text()
                .unwrap(),
            "saved-profile-ok"
        );
    };
    // Model traffic must not block the health check used during login readiness.
    let client = relay.client.clone();
    let request_url = format!("{}/responses", relay.base_url);
    let pending_request =
        thread::spawn(move || client.post(request_url).send().unwrap().text().unwrap());
    request_started_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(relay.health_pid(), Some(original_pid));
    release_tx.send(()).unwrap();
    assert_eq!(pending_request.join().unwrap(), "saved-profile-ok");
    relay.shutdown_worker();
    relay.wait_ready(Some(original_pid));
    forward(&relay);
    upstream_thread.join().unwrap();
    assert!(!unrelated.join("cs/profiles.json").exists());
}

#[cfg(windows)]
#[test]
fn startup_executable_uses_the_windowless_windows_subsystem() {
    let bytes = fs::read(env!("CARGO_BIN_EXE_cs-relay")).unwrap();
    let pe_offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let subsystem_offset = pe_offset + 24 + 68;
    let subsystem = u16::from_le_bytes(
        bytes[subsystem_offset..subsystem_offset + 2]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        subsystem, 2,
        "startup helper must be a GUI executable, not a console executable"
    );
}
