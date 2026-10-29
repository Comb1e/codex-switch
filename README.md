# codex-switch

`cs` switches named API providers for Codex Desktop through a local relay,
letting Codex keep its ChatGPT login while model requests use the selected
provider's API key. Use `cs gpt` to switch to the official OpenAI provider
directly, without the relay.

## Requirements

- Rust and Cargo to build from source.
- Codex Desktop with a ChatGPT login; custom providers also need an upstream HTTP(S) API key.
- PowerShell on Windows or Bash on Linux for the installer.

## Installation

PowerShell:

~~~powershell
.\install.ps1
~~~

Bash:

~~~bash
./install.sh
~~~

Both scripts install `cs` and its `cs-relay` background helper. If `cs` is not
found, add Cargo's bin directory to PATH (`%USERPROFILE%\.cargo\bin` on Windows
or `$HOME/.cargo/bin` on Linux). The Windows installer restores an existing
provider and startup entry after replacement.

## First commands

Inspect the current provider and all saved providers:

~~~text
cs status
~~~

Save a provider using the hidden API-key prompt, then activate it:

~~~text
cs add --name work --baseurl https://api.example.com/v1
cs work
cs status
~~~

Switch to the official OpenAI provider using your existing Codex login:

~~~text
cs gpt
~~~

This stops the relay and prevents it from restarting at login. Saved custom
profiles remain available: run `cs work` to use a saved provider again.
Restart Codex Desktop after switching so it reloads configuration. Running
`cs` without arguments restores the last selected mode, including `gpt`.
`gpt` is a reserved command and cannot be used as a custom profile name.

## Common checks

~~~text
cs status
cs current
cs list
cs --help
~~~

Development checks:

~~~bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --lib app::tests
cargo test --lib validation::tests
cargo test --test cli
~~~

## Documentation

- [Usage, generated configuration, authentication, and security](docs/usage.md)
- [Architecture and runtime workflows](docs/architecture.md)
- [Change history and references](docs/iteration.md)
