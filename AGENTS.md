# Project structure

`codex-switch` is a Rust project with one shared library and two binaries.
See [docs/architecture.md](docs/architecture.md) for system behavior and failure
paths, [docs/usage.md](docs/usage.md) for commands, and
[docs/iteration.md](docs/iteration.md) for history and references.

- `src/main.rs`: console `cs` entry point.
- `src/relay_startup.rs`: windowless Windows `cs-relay` entry point.
- `src/lib.rs`: shared CLI and startup setup and error handling.
- `src/cli.rs`: command parsing and API-key input.
- `src/app.rs`: profile operations, verified provider status, shared list
  formatting, official GPT mode, command locking, and activation rollback.
- `src/profile.rs`: versioned JSON profile storage and persisted relay/GPT mode.
- `src/codex.rs`: relay and official OpenAI TOML generation and verification.
- `src/relay.rs`: worker and supervisor lifecycle, startup registration, and
  HTTP forwarding.
- `src/paths.rs`: environment and platform path discovery.
- `src/atomic.rs`: atomic file writes and file permissions.
- `src/validation.rs`: profile names, base URLs, and API keys.
- `tests/cli.rs`: isolated CLI workflows and provider reporting.
- `tests/startup.rs`: relay startup, recovery, and request forwarding.
- `install.ps1`, `install.sh`: installation; `enable-relay.ps1`: key migration.

# Development

Keep the README as a quick start and update the architecture document when
behavior changes. Keep history only in `docs/iteration.md`. Reuse provider
verification and formatting across reporting commands, and never display API
keys. Extend existing focused tests for affected workflows; do not repeatedly
run the full suite for a local command change.
