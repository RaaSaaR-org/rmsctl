# CLAUDE.md

## What is this?

`rmsctl` is a Rust CLI client for the NeoDEM robot management server (RMS). It provides terminal access to fleet management, robot control, alerts, and telemetry — a lightweight alternative to the Tauri desktop app. Named after kubectl: "the CLI that controls RMS".

## Build & Development

```bash
cargo build --release          # Build release binary (target/release/rmsctl)
cargo build                    # Debug build
cargo fmt --check              # Check formatting
cargo clippy -- -D warnings    # Lint
```

## Configuration

Server URL resolution (first wins):
1. `--server` flag
2. `RMS_SERVER_URL` env var
3. `~/.config/rmsctl/config.json`
4. Default: `http://localhost:3001/api`

Auth token resolution:
1. `--token` flag
2. `RMS_TOKEN` env var
3. `token` field in config file

Note: env vars stay `RMS_*` because they describe the *RMS server* (the system being controlled), not the CLI tool — same convention as `KUBECONFIG` for `kubectl`.

Config file format (`~/.config/rmsctl/config.json`):
```json
{
  "server_url": "http://localhost:3001/api",
  "token": "optional-jwt"
}
```

## Architecture

```
src/
  main.rs          # Entry point, error display
  cli.rs           # clap derive definitions
  config.rs        # Config loading (flag > env > file > default)
  error.rs         # RmsError (thiserror) + hints
  client.rs        # ApiClient wrapping reqwest
  models.rs        # serde structs for API responses
  output.rs        # Colored terminal formatting
  commands/
    robots.rs      # list, show, register, remove, telemetry, command
    alerts.rs      # list, ack
    status.rs      # fleet overview dashboard
```

Internal Rust types (`RmsConfig`, `RmsError`, `RmsResult`) keep the `Rms` prefix because they refer to the RMS domain, not the binary name.

## Key Patterns

- Follows `mission-control` (mc) CLI patterns: clap derive, thiserror + hints, colored output
- Async throughout (reqwest + tokio)
- `--json` flag for machine-readable output on all commands
- Defensive deserialization: `#[serde(default)]` + `Option<T>` for API resilience

## NeoDEM Server

The CLI talks to the NeoDEM server REST API (default port 3001). Start the server:

```bash
cd ../robot-management-system/server && npm run dev
```

Dev mode has `AUTH_DISABLED=true` so no token is needed locally.
