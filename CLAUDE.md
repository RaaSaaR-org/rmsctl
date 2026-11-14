# CLAUDE.md

## What is this?

`rmsctl` is a Rust CLI client for the NeoDEM robot management server (RMS). It provides
terminal access to fleet management, robot control, safety and E-stop, incidents,
oversight, approvals, teleoperation, patrol, the data/training/model/deployment pipeline,
skills, processes, evaluation and compliance — a lightweight alternative to the Tauri
desktop app. Named after kubectl: "the CLI that controls RMS".

## Build & Development

```bash
cargo build --release          # Build release binary (target/release/rmsctl)
cargo build                    # Debug build
cargo fmt --check              # Check formatting
cargo clippy -- -D warnings    # Lint
cargo test                     # Unit tests + the upload and download flows against in-process mock servers
```

Both gates apply to new command modules too.

## Configuration

Server URL resolution (first wins):
1. `--server` flag
2. `RMS_SERVER_URL` env var
3. `~/.config/rmsctl/config.json`, then the platform config dir
4. Default: `http://localhost:3001/api`

Auth token resolution:
1. `--token` flag
2. `RMS_TOKEN` env var
3. `token` field in config file

Note: env vars stay `RMS_*` because they describe the *RMS server* (the system being
controlled), not the CLI tool — same convention as `KUBECONFIG` for `kubectl`.

Config file format:
```json
{
  "server_url": "http://localhost:3001/api",
  "token": "optional-jwt"
}
```

`config.rs` checks `~/.config/rmsctl/config.json` first on every platform, then falls
back to `dirs::config_dir()` (macOS: `~/Library/Application Support/rmsctl/config.json`).
The XDG path comes first deliberately: it is what the docs promise and what anyone
arriving from kubectl expects. Error hints print the resolved path via
`config::config_path_display()` rather than hard-coding one.

## Architecture

```
src/
  main.rs          # Entry point, error display, command dispatch
  cli.rs           # clap derive definitions (all 19 command groups)
  ctx.rs           # Ctx: global flags (--json, --yes, --page, --limit) + paging helpers
  config.rs        # Config loading (flag > env > file > default)
  error.rs         # RmsError (thiserror) + per-status hints
  client.rs        # ApiClient wrapping reqwest; typed + raw-Value variants of every verb
  models.rs        # serde structs shared by more than one module
  output.rs        # Colored terminal formatting, confirm gate, paging footer
  commands/
    robots.rs      # list, show, register, remove, telemetry, agent-mode, cameras, peers, command
    alerts.rs      # list, show, ack, history, delete, clear
    status.rs      # fleet overview dashboard (robots + alert counts + safety)
    safety.rs      # status, estop, reset, zone-estop, events
    incidents.rs   # list, show, dashboard, open, overdue, risk-matrix, evidence
    oversight.rs   # dashboard, fleet, anomalies, ack/resolve, manual-mode, verifications, logs
    approvals.rs   # list, show, pending, overdue, nearing, decide, cancel, escalate, metrics
    teleop.rs      # list, show, episodes, pause, resume, end
    patrol.rs      # routes, runs, findings, places, start, abort, ack/normal/escalate, promote
    updates.rs     # list, show, deployments, approve, deploy, rollback
    processes.rs   # list, show, instances, tasks, start, pause, resume, cancel, retry
    compliance.rs  # dashboard, deadlines, gaps, metrics, documents, training, inspections
    datasets.rs    # list, show, episodes, quality, stats, validate, push, flagged, flag (dispatches upload/download)
    dataset_upload.rs        # datasets upload | upload-status | upload-cancel (direct upload)
    dataset_upload_tests.rs  # its tests, incl. a mock server that speaks the upload contract
    dataset_download.rs      # datasets download — and models download (Kind::Model)
    dataset_verify.rs        # datasets verify — a local directory against the instance's file list
    dataset_verify_tests.rs  # its tests, on the download tests' mock server
    dataset_download_tests.rs # its tests, incl. a mock server that speaks the download contract
    training.rs    # list, show, active, queue, workers, start, cancel, retry, export, estimate
    models.rs      # list, show, lineage, compare, delete — and the dispatch of upload/files/download
    model_upload.rs        # models upload | upload-status | upload-cancel | files
    model_upload_tests.rs  # its tests, incl. a mock server that speaks the model contract
    deployments.rs # list, show, metrics, promote, rollback, cancel
    skills.rs      # list, show, for-robot, compatible, check, validate, publish, execute
    embodiments.rs # list, show
    eval.rs        # episodes, success-rate, episode, errors, rewards, compare, run-hardware
```

Internal Rust types (`RmsConfig`, `RmsError`, `RmsResult`) keep the `Rms` prefix because
they refer to the RMS domain, not the binary name.

## Key Patterns

- Follows `mission-control` (mc) CLI patterns: clap derive, thiserror + hints, colored output
- Async throughout (reqwest + tokio)
- Defensive deserialization: `#[serde(default)]` + `Option<T>` on everything that is not
  a guaranteed primary key. A required field the server omits is a hard runtime failure,
  and the server's shapes drift.
- Structs shared by two or more modules live in `models.rs`; everything else is declared
  locally in the owning module. The server's envelopes are inconsistent enough
  (`{robots}`, `{data}`, `{sessions}`, `{incidents}`, bare arrays, flat objects) that one
  struct per endpoint in a shared file was doing more harm than good.

### `--json` is a passthrough

Every command branches on `ctx.json` **before** deserializing and prints the untouched
server body via `client.get_value(...)` + `output::json_out(...)`. Routing `--json`
through a typed struct silently drops every field the struct omits — that was the single
most common bug in the pre-audit code, and it made `--json` unusable for scripting.

### The `--yes` gate

Anything that changes fleet or deployment state calls
`output::confirm(&format!("This will ..."), ctx.yes)` and returns early when it is false.
Without `--yes` the command describes the action and does nothing. The confirm text names
the specific robot / deployment / session, so a mistyped ID costs nothing. Read-only GETs
are never gated.

### Direct dataset upload

`datasets upload <dir>` sends a LeRobot directory straight into RMS, no Hugging Face in
between. The contract is the server's `docs/dataset-upload.md`; `dataset_upload.rs` is
written against it and nothing else. What must stay true when touching it:

- **Every regular file of the directory goes, and nothing is left out silently.** The
  purpose of upload + download is "push a dataset, delete the local copy, fetch it back
  later", so `scan_dir` takes everything and records each entry it passes over in
  `Scan.skipped` with a reason: the built-in junk (`junk_reason`: `.git`, `.DS_Store`,
  `__pycache__`, a top-level `.cache`, `*.rmsctl-part`), `--exclude` (the matcher is
  `model_upload::excluded`), empty directories, non-regular files, and with
  `--lerobot-only` everything beside the tree. Symlinks are followed; a dangling one is
  an error. The run without `--yes` prints the whole list. A new skip rule needs a
  reason string and a line in the README table.
- **A server that would drop files is refused.** A server before RMS TASK-500 answers
  the start request with the declared files it will not store (`ignored`). `start`
  then cancels the upload it just opened and fails naming them; `--lerobot-only` is
  the way to upload to such a server knowingly. Never carry on past a non-empty
  `ignored`.
- **`datasets verify` reads the directory by the same `ScanRules`** and compares it
  with `GET /datasets/:id/files` (path, size, SHA-256 where listed) without fetching a
  file. Any difference, in either direction, is a non-zero exit: it gates an `rm -rf`.
  A dataset that is not `ready` is a non-zero exit as well, files matching or not.
- **Nothing is left out silently, in any output mode.** Human output lists the
  left-out entries under `Left out`; `--json` prints the same list on stderr.
- **Never hold a part in memory.** A part is hashed in one pass over the file range and
  streamed from disk in a second (`reqwest::Body::wrap_stream`). Memory must not grow
  with file size, `--chunk-size` or `--parallel`. `Content-Length` is set by hand — a
  streamed body otherwise goes out chunked and the server answers 411.
- **The server is the source of truth for resume.** The only local state is a note
  mapping (server URL, directory) to a dataset id, under `~/.local/state/rmsctl/uploads/`
  (`RMSCTL_STATE_DIR` overrides). Which parts are stored is always read from
  `GET /datasets/:id/upload`. Never write into the dataset directory.
- **Branch on `code`, never on the sentence.** `classify()` decides what is retried;
  `UPLOAD_BUSY` waits without counting against `--retries`. Start is never re-sent after
  a network error or a proxy's 5xx (one without a `code`): it may have created the
  dataset. A refusal the server wrote itself created nothing and is retried.
- **Stop sending once the server answered.** Most refusals arrive before the body is
  read; the body stream ends itself then, or a 32 MiB part is pushed up a slow link for
  nothing (and the progress bar overshoots).
- **`--json` is the quiet mode.** No progress on stdout; the final `GET /datasets/:id`
  body is the only output. Exit code 0 only for `ready`.
- `upload-cancel` uses `POST …/upload/cancel` while an upload is open or failed, and
  `DELETE /datasets/:id` for an uploaded dataset that failed validation (the server
  answers 409 `UPLOAD_COMPLETED` to a cancel there).

The flow tests run against a hand-written HTTP/1.1 server in
`dataset_upload_tests.rs` (`Mock`); extend it when the contract grows rather than
mocking at the function level — the bugs in this feature live in headers, byte ranges
and ordering.

### Dataset download

`datasets download <id> [--out <dir>]` is the way back: the stored files of a dataset,
same relative paths, same bytes. The contract is the server's `docs/dataset-download.md`
(`GET /datasets/:id/files` — a cursor-paged manifest of `{path, size, etag?, sha256?}` —
and `GET /datasets/:id/files/content?path=` with `Range`). What must stay true:

- **Never hold a file in memory.** The response body goes to disk chunk by chunk.
- **A file has its real name only when it is whole.** Bytes land in `<name>.rmsctl-part`
  and are renamed after the size — and the SHA-256, when the manifest lists one — match.
  The hash is read back from disk, so a part continued from an earlier run is covered too.
- **The directory is the only state.** `--resume` skips files with the manifest's size and
  continues a part file with `Range: bytes=<len>-`; `--force` fetches everything again.
  A non-empty directory without either flag is refused before the manifest is fetched.
- **The manifest is not trusted with paths.** `safe_rel` checks every path before the
  first byte is written; nothing is created when one is refused.
- **Answers are checked, not assumed.** A `206` must carry the `Content-Range` that was
  asked for; a `200` to a ranged request restarts the file instead of appending; a size or
  ETag that differs from the manifest is "the dataset changed", fatal, not retried.
- **Retries:** an attempt that delivered bytes before it died does not count against
  `--retries` (it continues); one that delivered nothing does. `classify()` from the
  upload decides what is transient. The first file that cannot be fetched ends the run
  with an error — exit code 0 only when every listed file is on disk with its size.
- Read-only on the server, so no `--yes` gate. `--json` is quiet and ends with a summary
  object (there is no single server body to pass through).

The flow tests run against their own hand-written server in `dataset_download_tests.rs`
(paged manifest, `Range`, and faults: refusals, a connection dropped mid-body, corrupted
bytes, an ignored `Range`, a listing that lies about a size).

### Direct model upload and download

`models upload <dir> --version …` sends a checkpoint directory into the instance's model
store and registers it; `models download <id>` brings one back. The contract is the
server's `docs/model-upload.md`. Both are the dataset commands' engines pointed at the
model routes — the server speaks the same part protocol and hands files out the same way:

- **Upload** — `model_upload.rs` reuses `dataset_upload.rs` (`send_missing`, `build_plan`,
  `control`, `StateStore`, `Ui`, `Progress`, all `pub(super)`), which takes the upload's
  base path (`/datasets/:id/upload` or `/models/uploads/:id`).
- **Download** — `dataset_download.rs` takes a `Kind` (`Dataset` | `Model`) that says where
  the routes hang (`/datasets/:id` or `/models/versions/:id`, each with `/files` and
  `/files/content?path=`) and what the thing is called in messages. Everything under
  "Dataset download" holds for a model unchanged.

Everything listed under "Direct dataset upload" holds here too. What is different, and
must stay true:

- **Every file of the directory is a candidate.** A checkpoint has no fixed layout;
  `--exclude` (gitignore-style, `excluded()`) is the only filter.
- **The verdict is the registry.** The upload's `state` goes `completed` (a ModelVersion
  with the upload's id exists) or `failed` (nothing was registered). Exit code 0 only for
  `completed`. `upload-cancel` refuses a completed upload and names `models delete`.
- **The note key carries `#models`** (`note_server`), so one directory uploaded as a
  dataset and as a model to the same server keeps two notes.
- **A new kind of download is a `Kind` variant, not a copy.** Paths and nouns go through
  `Kind`; nothing else in the engine may name a dataset.

### Pagination honesty

`output::count_paged(label, shown, total, total_pages)` is used instead of a bare count.
When the envelope's `total` exceeds the number of rows printed it says so and names the
flag that fetches the rest. The old `count_label` printed `total` under a single page, so
`alerts list` claimed 75 rows while showing 50.

Page-size query keys differ per route: most read `pageSize` (`ctx.paging()`), while
`/datasets` and the history routes read `limit` (`ctx.paging_limit()`).

### Status vocabulary

`output::status_indicator` and `models::Robot::health()` both know the safety-critical
states. A robot in `protective_stop` / `emergency_stop` must never render as normal and
must never count toward availability — the pre-audit dashboard folded every unrecognised
status into "Other" and computed availability as `online / total`, so a fleet with a
latched E-stop read as merely idle.

An expected robot — `offline` with `metadata.provisioning` = `awaiting_agent` or
`arriving`, the rule the web UI uses — reads "not connected" / "arriving"
(`Robot::expected_label`) and is left out of the availability denominator. Only that
exact pair qualifies: the server drops the key when an agent registers, so a robot that
dropped out stays `offline` and counts. A latched E-stop still wins over it.

## NeoDEM Server

The CLI talks to the NeoDEM server REST API (default port 3001). Start the server:

```bash
cd ../robot-management-system/server && npm run dev
```

Dev mode has `AUTH_DISABLED=true` so no token is needed locally.

Health check lives at the root, not under `/api`: `curl http://localhost:3001/health`
returns 200 while `/api/health` is a 404.

## Coverage

The server mounts 61 distinct `/api/*` route groups (plus `/metrics` and
`/.well-known/a2a` outside `/api`). `rmsctl` covers 19 of them:

`robots` · `alerts` · `safety` · `incidents` · `oversight` · `approvals` ·
`teleoperation` · `patrol` · `updates` · `processes` · `compliance/tracker` ·
`datasets` · `training` · `models` · `deployments` · `skills` · `embodiments` ·
`evaluation`

(`robots command` posts to `/robots/:id/command`; the separate `/api/command` group —
`/interpret`, `/history` — is not covered.)

Deliberately **not** covered:
- Worker/agent-auth only, not an operator surface: `training/workers/*` internals,
  `twin/workers`, `a2a/*` (inter-agent protocol), robot camera MJPEG streams,
  `teleoperation/sessions/:id/frame(s)` (VR data plane), `.well-known/a2a`
- Broken or uninitialised on a dev box: `storage` (503, RustFS not initialised),
  `sensor-scans` (500), `motion-clips` (500), `compliance/logs` (500)
- Lower terminal value, available if wanted: `zones`, `simulation`, `isaac-lab`,
  `explainability`, `federated`, `marketplace`, `security`, `settings`, `team`,
  `tenants`, `gdpr`, `digital-twins`, `contributions`, `synthetic*`, `curation`,
  `active-learning`, `tour`, `uncertainty`, `notification-templates`, `scan-sessions`,
  `training-docs`

Route groups whose prefix 404s with no index route — a command must target a subpath:
`compliance`, `compliance/tracker`, `gdpr`, `explainability`, `evaluation`, `isaac-lab`,
`curation`, `active-learning`, `marketplace`, `federated`, `training-docs`,
`scan-sessions`, `tour`, `uncertainty`, `command`.

When adding a command, curl the endpoint first and model the envelope you actually get.
Drift here is invisible until a user hits it: `models` shipped pointing at an MLflow-style
`/models/registry` that never existed on this server, and every one of its subcommands
404'd.
