# Iteration history

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
