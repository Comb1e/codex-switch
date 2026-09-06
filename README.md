# cs

`cs` is a small Windows and Linux command-line tool for saving and switching named
Codex API endpoint and API-key profiles.

It updates the user-level Codex files documented in the official OpenAI
[configuration reference](https://developers.openai.com/codex/config-reference)
and [authentication guide](https://developers.openai.com/codex/auth). Existing
unrelated `config.toml` settings, comments, and `auth.json` fields are preserved.

## Install

Rust and Cargo are required to build from source.

PowerShell:

```powershell
.\install.ps1
```

Bash:

```bash
./install.sh
```

Both scripts run `cargo install --path <repository> --force`. If `cs` is not found
after installation, add Cargo's bin directory to `PATH` (`$HOME/.cargo/bin` on
Linux or `%USERPROFILE%\.cargo\bin` on Windows).

## Usage

Save a profile and activate it:

```powershell
# PowerShell
cs add --name work --baseurl "https://proxy.example.com/v1" --apikey "$env:OPENAI_API_KEY"
cs work
```

```bash
# Bash
cs add --name work --baseurl "https://proxy.example.com/v1" --apikey "$OPENAI_API_KEY"
cs work
```

For less exposure through process arguments, pipe the key through standard input:

```powershell
$env:OPENAI_API_KEY | cs add --name work --baseurl "https://proxy.example.com/v1" --apikey-stdin
```

```bash
printf '%s' "$OPENAI_API_KEY" | cs add --name work --baseurl "https://proxy.example.com/v1" --apikey-stdin
```

When neither `--apikey` nor `--apikey-stdin` is supplied in an interactive
terminal, `cs` prompts for the key without echoing it.

Other commands:

```text
cs list
cs current
cs remove work
cs remove work --force
cs --help
cs --version
```

Adding an existing name requires `--force`. Overwriting an active profile clears
its active marker until it is activated again. Removing an active profile also
requires `--force`; this removes only the saved profile and leaves the currently
applied Codex files unchanged.

## Files and security

Profiles are stored in `%LOCALAPPDATA%\cs\profiles.json` on Windows and
`${XDG_CONFIG_HOME:-~/.config}/cs/profiles.json` on Linux. `CODEX_HOME` selects
the Codex directory and defaults to `~/.codex`.

API keys are deliberately stored as plaintext, as they are in Codex's file-backed
`auth.json`. Treat both files as secrets. On Linux, `cs` creates its data
directory with mode `0700` and secret files with mode `0600`. On Windows, files
inherit the ACL of the user's local application-data directory.

Activation uses atomic file replacement and rollback. It sets these Codex values:

```toml
model_provider = "openai"
openai_base_url = "https://proxy.example.com/v1"
cli_auth_credentials_store = "file"
```

It also sets `auth_mode` to `apikey` and updates `OPENAI_API_KEY` in `auth.json`.
Already-running Codex processes must be restarted to read the new settings.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
