# codex-switch

cs lets Codex Desktop stay signed in with a ChatGPT account while model
requests use a separate relay API key.

Codex has one shared authentication cache. Switching that cache to
auth_mode = "apikey" makes the desktop app stop treating the ChatGPT login as
active. cs avoids that conflict:

1. Codex keeps the ChatGPT tokens in auth.json.
2. Codex sends model requests to a loopback-only relay at 127.0.0.1:14556.
3. The relay replaces the ChatGPT Authorization header with the API key from
   the active cs profile, then forwards the request to the configured upstream.

The relay never sends the ChatGPT access token to the upstream server.

## Install

Rust and Cargo are required to build from source.

PowerShell:

~~~powershell
.\install.ps1
~~~

Bash:

~~~bash
./install.sh
~~~

Both scripts install `cs` and its `cs-relay` background helper with Cargo.
The Windows installer builds the locked dependencies first, stops only the
installed relay processes for replacement, migrates an existing startup entry,
and restarts the last selected profile automatically.
If cs is not found
afterward, add Cargo's bin directory to PATH (%USERPROFILE%\.cargo\bin on
Windows or $HOME/.cargo/bin on Linux).

## Configure the relay

Save the relay key without echoing it in the terminal, then activate the profile:

~~~powershell
$env:RELAY_API_KEY | cs add --name transit --baseurl "http://154.201.72.176:8080/" --apikey-stdin
cs transit
# Later, reuse the last selected profile:
cs
~~~

For the migration sequence where the relay key is currently stored in
auth.json, close Codex Desktop and run:

~~~powershell
.\enable-relay.ps1
~~~

The script imports that key into the transit profile without displaying it. It
does not modify auth.json. Replace auth.json with your ChatGPT login file after
the script completes.

Activation stops the currently managed relay, updates ~/.codex/config.toml,
starts a fresh background relay for the selected profile, and registers
CodexSwitchRelay under the current user's Windows startup entries. It does not
create, edit, or delete ~/.codex/auth.json.

After activation, place the ChatGPT login file produced by
[codex-auth-helper](https://github.com/zhishile/codex-auth-helper) at
~/.codex/auth.json, or use Codex's normal ChatGPT login. Close and reopen Codex
Desktop after changing the configuration or authentication file.

The last successfully selected profile is remembered across restarts. Running
`cs` without arguments activates that profile; on a new installation, select one
with `cs <name>` first. Removing or replacing the active profile clears the default.

When the same Windows user signs in, the windowless `cs-relay.exe` supervisor
automatically starts the remembered profile. It stays in the background and
restarts the relay within a few seconds if the worker exits. No terminal needs
to remain open. Duplicate launches reuse the existing supervisor. Startup uses
the saved absolute profile directory, so it does not depend on the login
working directory or terminal environment. Startup entries contain no API keys.
Failures are recorded in `relay.stderr.log` beside `profiles.json`, with retries
backing off to 30 seconds. This is a per-user background service, active while
the user is signed in; it is not a system service running before login.

## Generated Codex configuration

The effective provider configuration is:

~~~toml
model_provider = "codex_switch_relay"
disable_response_storage = true
network_access = "enabled"
windows_wsl_setup_acknowledged = true

[model_providers.codex_switch_relay]
name = "Codex Switch Relay"
base_url = "http://127.0.0.1:14556/"
wire_api = "responses"
requires_openai_auth = true

[features]
goals = true
remote_compaction_v2 = true
~~~

The provider base_url intentionally points to the local relay. The real upstream
URL remains in the selected profile. Pointing Codex directly at the upstream
while requires_openai_auth = true would send the ChatGPT bearer token instead of
the relay API key.

Unrelated TOML settings, tables, and comments are preserved. Conflicting fields
inside [model_providers.codex_switch_relay] are replaced. The legacy
[model_providers.OpenAI] table from codex-switch 0.2.0 is removed because
`openai` is a reserved built-in provider ID and provider IDs are normalized
case-insensitively by Codex.

## Profile commands

~~~text
cs add --name NAME --baseurl URL --apikey KEY
cs add --name NAME --baseurl URL --apikey-stdin
cs NAME
cs list
cs current
cs remove NAME
cs remove NAME --force
cs --help
cs --version
~~~

Adding an existing name requires --force. Removing an active profile also
requires --force.

Every cs NAME activation restarts the managed relay, so the newly selected base
URL and API key take effect immediately in the proxy. Restart Codex Desktop
after switching so it reloads config.toml.

## Files and security

Profiles are stored in %LOCALAPPDATA%\cs\profiles.json on Windows and
${XDG_CONFIG_HOME:-~/.config}/cs/profiles.json on Linux. CODEX_HOME selects the
Codex directory and defaults to ~/.codex.

API keys are stored as plaintext in the profile store. On Linux, cs creates the
data directory with mode 0700 and the profile store with mode 0600. On Windows,
files inherit the ACL of the user's local application-data directory. Relay logs
contain errors only, not request bodies or authorization headers.

The configured upstream in the example uses plain HTTP. API keys, prompts, tool
outputs, and responses are not encrypted between the local relay and that
server. Use HTTPS when the relay service supports it.

Codex's official
[authentication guide](https://developers.openai.com/codex/auth) documents that
requires_openai_auth = true can use ChatGPT authentication with a custom
provider. The
[configuration reference](https://developers.openai.com/codex/config-reference)
documents the provider fields used here.

## Development

The system architecture, data flow, persistence model, and key command
workflows are documented in [docs/architecture.md](docs/architecture.md).

~~~bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
~~~
