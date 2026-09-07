use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use tiny_http::{Header, Request, Response, Server, StatusCode};
use url::Url;

use crate::paths::AppPaths;
use crate::profile::ProfileStore;

pub const RELAY_HOST: &str = "127.0.0.1";
pub const RELAY_PORT: u16 = 14556;
pub const PROVIDER_ID: &str = "codex_switch_relay";

const HEALTH_PATH: &str = "/__codex_switch_health";
const HEALTH_MARKER: &str = "codex-switch-relay";
const SHUTDOWN_PATH: &str = "/__codex_switch_shutdown";

pub trait RelayManager {
    fn restart(&self, paths: &AppPaths) -> Result<()>;
}

#[derive(Clone, Copy)]
pub struct RealRelayManager;

impl RelayManager for RealRelayManager {
    fn restart(&self, paths: &AppPaths) -> Result<()> {
        #[cfg(debug_assertions)]
        if std::env::var_os("CS_TEST_SKIP_RELAY_START").is_some() {
            return Ok(());
        }
        install_autostart()?;
        stop_running()?;

        let executable = std::env::current_exe().context("could not locate the cs executable")?;
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&paths.relay_stdout_file)
            .with_context(|| {
                format!(
                    "could not open relay log {}",
                    paths.relay_stdout_file.display()
                )
            })?;
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&paths.relay_stderr_file)
            .with_context(|| {
                format!(
                    "could not open relay log {}",
                    paths.relay_stderr_file.display()
                )
            })?;

        let mut command = Command::new(executable);
        command
            .arg("__relay")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }

        let mut child = command.spawn().context("could not start the local relay")?;
        for _ in 0..40 {
            if is_running() {
                return Ok(());
            }
            if let Some(status) = child
                .try_wait()
                .context("could not inspect the local relay process")?
            {
                bail!(
                    "local relay exited with {status}; see {}",
                    paths.relay_stderr_file.display()
                );
            }
            thread::sleep(Duration::from_millis(100));
        }

        bail!(
            "local relay did not become ready; see {}",
            paths.relay_stderr_file.display()
        )
    }
}

pub fn base_url() -> String {
    format!("http://{RELAY_HOST}:{RELAY_PORT}/")
}

pub fn serve(paths: AppPaths) -> Result<()> {
    let address = format!("{RELAY_HOST}:{RELAY_PORT}");
    let server = Server::http(&address)
        .map_err(|error| anyhow::anyhow!("could not bind local relay to {address}: {error}"))?;
    let client = Client::builder()
        .redirect(Policy::none())
        .build()
        .context("could not create the relay HTTP client")?;

    for request in server.incoming_requests() {
        if request.url() == HEALTH_PATH {
            respond_json(
                request,
                200,
                format!(r#"{{"ok":true,"service":"{HEALTH_MARKER}"}}"#),
            );
            continue;
        }
        if request.url() == SHUTDOWN_PATH {
            respond_json(request, 200, r#"{"ok":true}"#.into());
            break;
        }
        if let Err(error) = forward(request, &client, &paths) {
            eprintln!("relay request failed: {error:#}");
        }
    }
    Ok(())
}

fn forward(mut request: Request, client: &Client, paths: &AppPaths) -> Result<()> {
    let store = ProfileStore::load(&paths.store_file)?;
    let active = store
        .active
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no relay profile is active"))?;
    let profile = store
        .profiles
        .get(active)
        .ok_or_else(|| anyhow::anyhow!("active relay profile '{active}' no longer exists"))?;
    let target = target_url(&profile.base_url, request.url())?;
    let method = reqwest::Method::from_bytes(request.method().as_str().as_bytes())
        .context("request method is invalid")?;

    let mut builder = client.request(method, target).bearer_auth(&profile.api_key);
    for header in request.headers() {
        let name = header.field.to_string();
        if is_request_hop_header(&name) {
            continue;
        }
        builder = builder.header(name, header.value.as_str());
    }

    let mut body = Vec::new();
    request
        .as_reader()
        .read_to_end(&mut body)
        .context("could not read the Codex request body")?;

    let upstream = match builder.body(body).send() {
        Ok(response) => response,
        Err(error) => {
            respond_json(
                request,
                502,
                r#"{"error":"relay upstream request failed"}"#.into(),
            );
            return Err(error).context("upstream request failed");
        }
    };

    let status = StatusCode(upstream.status().as_u16());
    let content_length = upstream.content_length().map(|length| length as usize);
    let headers = upstream
        .headers()
        .iter()
        .filter(|(name, _)| !is_response_hop_header(name.as_str()))
        .filter_map(|(name, value)| {
            Header::from_bytes(name.as_str().as_bytes(), value.as_bytes()).ok()
        })
        .collect();
    request
        .respond(Response::new(
            status,
            headers,
            upstream,
            content_length,
            None,
        ))
        .context("could not stream the upstream response to Codex")
}

fn target_url(base_url: &str, request_path: &str) -> Result<Url> {
    let joined = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        request_path.trim_start_matches('/')
    );
    Url::parse(&joined).with_context(|| format!("could not build upstream URL from {base_url}"))
}

fn is_request_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "connection"
            | "content-length"
            | "host"
            | "proxy-authorization"
            | "proxy-connection"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_response_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection" | "content-length" | "keep-alive" | "transfer-encoding" | "upgrade"
    )
}

fn respond_json(request: Request, status: u16, body: String) {
    let response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes("content-type", "application/json").unwrap());
    let _ = request.respond(response);
}

fn is_running() -> bool {
    probe(HEALTH_PATH)
        .map(|response| response.starts_with("HTTP/1.1 200") && response.contains(HEALTH_MARKER))
        .unwrap_or(false)
}

fn stop_running() -> Result<()> {
    if !is_running() {
        return Ok(());
    }
    let response = probe(SHUTDOWN_PATH).context("could not ask the existing relay to stop")?;
    if !response.starts_with("HTTP/1.1 200") {
        bail!("existing relay refused the shutdown request");
    }
    for _ in 0..40 {
        if !port_is_open() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    bail!("existing relay did not stop within two seconds")
}

fn probe(path: &str) -> Result<String> {
    let address = SocketAddr::from(([127, 0, 0, 1], RELAY_PORT));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(150))
        .context("could not connect to the local relay")?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {RELAY_HOST}:{RELAY_PORT}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .context("could not write to the local relay")?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("could not read from the local relay")?;
    Ok(response)
}

fn port_is_open() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], RELAY_PORT));
    TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
}

#[cfg(windows)]
fn install_autostart() -> Result<()> {
    let executable = std::env::current_exe().context("could not locate the cs executable")?;
    let command_line = format!("\"{}\" __relay", executable.display());
    let status = Command::new("reg.exe")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "CodexSwitchRelay",
            "/t",
            "REG_SZ",
            "/d",
            &command_line,
            "/f",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("could not register relay startup")?;
    if !status.success() {
        bail!("could not register relay startup in the current user's Run key");
    }
    Ok(())
}

#[cfg(not(windows))]
fn install_autostart() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::thread;

    use reqwest::blocking::Client;
    use tempfile::tempdir;
    use tiny_http::{Header, Response, Server};

    use super::{forward, target_url};
    use crate::atomic::{FileSensitivity, FileWriter, RealFileWriter};
    use crate::paths::AppPaths;
    use crate::profile::{Profile, ProfileStore};

    #[test]
    fn joins_root_and_versioned_base_urls() {
        assert_eq!(
            target_url("http://example.test:8080/", "/v1/responses")
                .unwrap()
                .as_str(),
            "http://example.test:8080/v1/responses"
        );
        assert_eq!(
            target_url("https://example.test/v1", "/responses?stream=true")
                .unwrap()
                .as_str(),
            "https://example.test/v1/responses?stream=true"
        );
    }

    #[test]
    fn replaces_chatgpt_authorization_and_streams_the_response() {
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let upstream_url = format!("http://{}/", upstream.server_addr());
        let upstream_thread =
            thread::spawn(move || {
                let mut request = upstream.recv().unwrap();
                assert_eq!(request.url(), "/v1/responses");
                let authorization = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("authorization"))
                    .unwrap();
                assert_eq!(authorization.value.as_str(), "Bearer relay-secret");
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                assert_eq!(body, r#"{"model":"test"}"#);
                request
                    .respond(Response::from_string("data: relay-ok\n\n").with_header(
                        Header::from_bytes("content-type", "text/event-stream").unwrap(),
                    ))
                    .unwrap();
            });

        let temp = tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path());
        let store = ProfileStore {
            version: 1,
            active: Some("relay".into()),
            profiles: BTreeMap::from([(
                "relay".into(),
                Profile {
                    base_url: upstream_url,
                    api_key: "relay-secret".into(),
                },
            )]),
        };
        RealFileWriter
            .write(
                &paths.store_file,
                &store.encode().unwrap(),
                FileSensitivity::Secret,
            )
            .unwrap();

        let relay = Server::http("127.0.0.1:0").unwrap();
        let relay_url = format!("http://{}/v1/responses", relay.server_addr());
        let relay_paths = paths.clone();
        let relay_thread = thread::spawn(move || {
            let request = relay.recv().unwrap();
            forward(request, &Client::new(), &relay_paths).unwrap();
        });

        let response = Client::new()
            .post(relay_url)
            .bearer_auth("chatgpt-access-token")
            .body(r#"{"model":"test"}"#)
            .send()
            .unwrap();
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        assert_eq!(response.text().unwrap(), "data: relay-ok\n\n");

        relay_thread.join().unwrap();
        upstream_thread.join().unwrap();
        assert!(
            !fs::read_to_string(paths.store_file)
                .unwrap()
                .contains("chatgpt-access-token")
        );
    }
}
