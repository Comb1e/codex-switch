# Architecture

This document describes the structure and runtime behavior of `codex-switch`
(`cs`). It is the source of truth for module responsibilities, persistent data,
and the workflows that change or consume a profile.

## System architecture

`cs` is a single Rust binary with two execution modes:

- An interactive command-line process manages profiles and Codex configuration.
- The hidden `cs __relay` process listens only on `127.0.0.1:14556` and proxies
  Codex model requests to the active profile's upstream API.

```mermaid
flowchart LR
    User[User] --> CLI[CLI process<br/>cs]
    CLI --> Store[(profiles.json)]
    CLI --> Config[(Codex config.toml)]
    CLI --> Startup[Windows user startup entry]
    CLI --> Relay[Relay process<br/>cs __relay]

    Desktop[Codex Desktop] -->|ChatGPT bearer token| Relay
    Relay -->|read active profile| Store
    Relay -->|profile API key| Upstream[Configured upstream API]
    Upstream -->|streamed response| Relay
    Relay -->|streamed response| Desktop

    Desktop -. reads .-> Config
    Desktop -. reads, never modified by cs .-> Auth[(Codex auth.json)]
```

The local relay is the trust boundary between Codex's ChatGPT authentication
and the configured upstream. It discards the incoming `Authorization` header,
adds the active profile's API key, and never forwards the ChatGPT access token.

## Module responsibilities

```mermaid
flowchart TD
    Main[main.rs<br/>bootstrap and error reporting] --> Cli[cli.rs<br/>argument parsing and API-key input]
    Main --> Paths[paths.rs<br/>platform path discovery]
    Main --> App[app.rs<br/>command orchestration]

    Cli -->|Action| App
    App --> Validation[validation.rs<br/>profile input validation]
    App --> Profile[profile.rs<br/>versioned profile store]
    App --> Codex[codex.rs<br/>TOML preparation and verification]
    App --> Atomic[atomic.rs<br/>atomic writes and permissions]
    App --> Relay[relay.rs<br/>relay lifecycle and HTTP proxy]

    Profile --> Atomic
    Codex --> Relay
    Relay --> Paths
    Relay --> Profile
```

| Module | Responsibility | Important interface |
| --- | --- | --- |
| `main` | Parse, construct dependencies, execute, and print results | `run` |
| `cli` | Convert command-line syntax into domain actions without printing secrets | `Cli::into_action`, `Action` |
| `app` | Coordinate commands, locking, activation, verification, and rollback | `App::execute` |
| `profile` | Load, validate the version of, encode, and save the JSON profile store | `ProfileStore` |
| `codex` | Preserve unrelated TOML while installing and checking the relay provider | `prepare`, `file_matches` |
| `relay` | Restart the managed process, expose health controls, and proxy requests | `RelayManager`, `serve` |
| `atomic` | Write files atomically and apply sensitive-file permissions | `FileWriter` |
| `paths` | Resolve all platform-dependent files from environment and OS directories | `AppPaths` |
| `validation` | Enforce profile name, URL, and API-key constraints | `validate_*` |

`FileWriter` and `RelayManager` are dependency-injection boundaries. Production
uses `RealFileWriter` and `RealRelayManager`; tests replace them to exercise
write failures, relay failures, and rollback behavior deterministically.

## Persistent data

```mermaid
flowchart TB
    Env[Environment and OS directories] --> Paths[AppPaths::discover]
    Paths --> ProfilePath[Platform config directory/cs/profiles.json]
    Paths --> LockPath[Platform config directory/cs/.lock]
    Paths --> Logs[Platform config directory/cs/relay.*.log]
    Paths --> CodexHome[CODEX_HOME or user home/.codex]
    CodexHome --> Config[config.toml]
    CodexHome --> Auth[auth.json]

    App[App commands] -->|exclusive lock| LockPath
    App -->|secret atomic write| ProfilePath
    App -->|regular atomic write| Config
    Relay[Relay process] -->|read for each request| ProfilePath
    Relay -->|errors only| Logs
    App -. never reads or writes .-> Auth
    Relay -. never reads or writes .-> Auth
```

`profiles.json` is a versioned JSON document containing the active profile and
a sorted map of names to base URLs and plaintext API keys. On Unix, the store
directory is mode `0700` and the store is mode `0600`; on Windows it inherits
the current user's local application-data ACL. `config.toml` contains only the
loopback provider configuration, never the upstream URL or profile API key.

All user-facing commands except `__relay` acquire the same exclusive `.lock`
file. Writes use a temporary file plus atomic replacement, and activation keeps
the previous bytes so both modified files can be restored if any later step
fails.

## Command workflow

```mermaid
flowchart TD
    Start[Process starts] --> Parse[Cli::parse and into_action]
    Parse --> RelayCheck{Action is Relay?}
    RelayCheck -->|yes| Serve[relay::serve]
    RelayCheck -->|no| Lock[Acquire exclusive store lock]
    Lock --> Dispatch{Action}

    Dispatch -->|Add| Add[Validate input<br/>load store<br/>insert profile<br/>atomic save]
    Dispatch -->|List| List[Load store<br/>verify configured provider<br/>format profiles]
    Dispatch -->|Current| Current[Load store<br/>verify configured provider<br/>print name or unmanaged]
    Dispatch -->|Remove| Remove[Validate name<br/>enforce force rule<br/>atomic save]
    Dispatch -->|Activate| Activate[Run activation state machine]

    Add --> Output[Return display-safe output]
    List --> Output
    Current --> Output
    Remove --> Output
    Activate --> Output
```

`current` and the active marker shown by `list` are deliberately verified
values. A name is active only when it exists in the store and `config.toml`
still exactly matches the required relay configuration. External edits therefore
produce `unmanaged` rather than a stale active result.

## Activation state machine

Activation is the only multi-file transaction. Its state machine records the
last attempted phase for diagnostics and determines when rollback is required.

```mermaid
stateDiagram-v2
    [*] --> Locked
    Locked --> Loaded: load profile store
    Loaded --> Validated: validate selected profile
    Validated --> Staged: snapshot old files and prepare new bytes
    Staged --> RecordingActive
    RecordingActive --> ActiveRecorded: write profiles.json
    ActiveRecorded --> WritingConfig
    WritingConfig --> ConfigWritten: write config.toml
    ConfigWritten --> Verifying
    Verifying --> Verified: config matches required provider
    Verified --> RestartingRelay
    RestartingRelay --> RelayReady: relay health check passes
    RelayReady --> Done
    Done --> [*]

    RecordingActive --> Rollback: any error after writes begin
    ActiveRecorded --> Rollback: error
    WritingConfig --> Rollback: error
    ConfigWritten --> Rollback: error
    Verifying --> Rollback: error or mismatch
    Verified --> Rollback: error
    RestartingRelay --> Rollback: error
    Rollback --> Failed: restore or remove both files
```

The workflow stages the new store and TOML entirely in memory before writing.
It then records the active profile, writes the Codex configuration, verifies
the file from disk, and restarts the relay. Failure at or after
`RecordingActive` restores the original `profiles.json` and `config.toml`
contents; files that did not previously exist are removed.

## Relay request flow

```mermaid
sequenceDiagram
    participant C as Codex Desktop
    participant R as Loopback relay
    participant S as profiles.json
    participant U as Upstream API

    C->>R: HTTP request with ChatGPT Authorization
    R->>S: Load store and resolve active profile
    S-->>R: base_url and api_key
    R->>R: Join URL and remove hop-by-hop/auth headers
    R->>U: Request with Bearer profile.api_key
    alt Upstream request succeeds
        U-->>R: Status, allowed headers, streaming body
        R-->>C: Stream response unchanged
    else Connection or request fails
        R-->>C: 502 JSON response
        R->>R: Log error without body or credentials
    end
```

Redirect following is disabled so credentials are not automatically carried to
another origin. Request headers excluded from forwarding are `authorization`,
connection-specific headers, `host`, content length, and proxy authorization.
Equivalent hop-by-hop response headers are also removed. The body is buffered
for the upstream request; the upstream response body is streamed back to Codex.

## Relay lifecycle

```mermaid
flowchart TD
    Activate[Successful configuration write] --> Register[Register current-user startup<br/>Windows only]
    Register --> Probe{Managed relay healthy?}
    Probe -->|yes| Shutdown[POST internal shutdown endpoint]
    Shutdown --> Closed[Wait for port to close]
    Probe -->|no| Spawn
    Closed --> Spawn[Spawn cs __relay with log files]
    Spawn --> Ready{Health marker received?}
    Ready -->|yes, within 4 seconds| Success[Activation succeeds]
    Ready -->|child exits or timeout| Failure[Activation fails and files roll back]
```

The health and shutdown endpoints are bound to loopback with the relay and are
used only for lifecycle management. The health probe verifies a service marker,
not merely that some process has opened port `14556`.

## Change checklist

When behavior changes, update the corresponding diagram and description here:

- A module or dependency boundary changes: update **System architecture** and
  **Module responsibilities**.
- A file, environment variable, or persistence rule changes: update
  **Persistent data**.
- Command routing or semantics change: update **Command workflow**.
- Activation phases, ordering, or rollback behavior change: update
  **Activation state machine**.
- Proxy headers, authentication, streaming, or lifecycle change: update
  **Relay request flow** or **Relay lifecycle**.
