# Iteration history

## 2026-10-06 — Provider status

Added `cs status` to show the current verified provider and every saved profile
with its base URL and active marker. The summary and table use the same store
snapshot and configuration verification; table formatting is shared with
`cs list`. `status` is reserved as a command name. The existing `current`,
`list`, switching, and resume behavior is preserved.

Validation extends the existing CLI lifecycle coverage to include multiple
providers, inactive and active selections, missing and malformed configuration,
read-only behavior, and absence of API keys in output. Command parsing and
reserved-name rejection are checked separately. The README is now a quick start;
detailed usage is in `usage.md`, and history is kept here.

Project reference used: [rustup command examples](https://github.com/rust-lang/rustup/blob/master/doc/user-guide/src/examples.md).
Its `rustup show` command informed the explicit current-selection summary.

## 2026-09-14 — 0.2.2

The previous version registered the console worker (`cs __relay`) directly in
the Windows Run key. Login therefore opened a persistent terminal, and closing
that terminal could terminate the relay. Activation detached its worker, but
login bypassed that launch path, its log redirection, and readiness checks.
There was no supervisor to recover a stopped worker. Startup rediscovered the
profile directory from its environment instead of preserving the selected
directory. The stored active profile existed, but bare `cs` displayed help
instead of restoring it. Serial request forwarding could also block startup
health checks behind a model response.
The Windows installer also attempted to overwrite the running worker executable,
which Windows rejected with access denied.

This update shares the relay lifecycle through a library, installs a native
windowless supervisor, preserves the absolute data directory, and redirects
worker output to logs. A singleton lock prevents competing supervisors; the
existing command lock serializes worker recovery with profile switching.
The supervisor retries failed starts with capped backoff and restarts exited
workers. Health requests remain responsive during model requests. Bare `cs`
uses the last successful selection, and the Windows installer migrates existing
startup entries and restarts the saved relay without another named activation.
The installer builds with locked dependencies before briefly stopping its own
installed processes for replacement, and attempts to restore the prior profile
if installation fails.

Validation covers isolated login startup, saved-profile forwarding despite a
different environment, duplicate supervisor launches, recovery after worker
exit, health during a pending upstream request, the Windows GUI executable
subsystem, and restoring the last profile from a new CLI process.
