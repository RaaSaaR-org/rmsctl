use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "rmsctl",
    about = "NeoDEM robot management CLI",
    version,
    max_term_width = 100
)]
pub struct Cli {
    /// Server URL (default: http://localhost:3001/api)
    #[arg(long, env = "RMS_SERVER_URL", global = true)]
    pub server: Option<String>,

    /// JWT auth token
    #[arg(long, env = "RMS_TOKEN", global = true)]
    pub token: Option<String>,

    /// Output the server payload as JSON instead of formatted text
    #[arg(long, global = true)]
    pub json: bool,

    /// Execute state-changing commands instead of describing them
    #[arg(long, global = true)]
    pub yes: bool,

    /// Page of results to fetch (list commands)
    #[arg(long, global = true, value_name = "N")]
    pub page: Option<u32>,

    /// Results per page (list commands)
    #[arg(long, global = true, value_name = "N")]
    pub limit: Option<u32>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage robots
    Robots {
        #[command(subcommand)]
        cmd: RobotsCommand,
    },
    /// Manage alerts
    Alerts {
        #[command(subcommand)]
        cmd: AlertsCommand,
    },
    /// Show fleet status overview
    Status,
    /// Emergency stop and safety state
    Safety {
        #[command(subcommand)]
        cmd: SafetyCommand,
    },
    /// Incident triage
    Incidents {
        #[command(subcommand)]
        cmd: IncidentsCommand,
    },
    /// Human oversight: anomalies, manual mode, verifications
    Oversight {
        #[command(subcommand)]
        cmd: OversightCommand,
    },
    /// Approval workflows
    Approvals {
        #[command(subcommand)]
        cmd: ApprovalsCommand,
    },
    /// Teleoperation sessions
    Teleop {
        #[command(subcommand)]
        cmd: TeleopCommand,
    },
    /// Patrol routes, runs and findings
    Patrol {
        #[command(subcommand)]
        cmd: PatrolCommand,
    },
    /// Robot OTA updates
    Updates {
        #[command(subcommand)]
        cmd: UpdatesCommand,
    },
    /// Process definitions, instances and tasks
    Processes {
        #[command(subcommand)]
        cmd: ProcessesCommand,
    },
    /// Compliance tracker
    Compliance {
        #[command(subcommand)]
        cmd: ComplianceCommand,
    },
    /// Browse, upload and validate datasets
    Datasets {
        #[command(subcommand)]
        cmd: DatasetsCommand,
    },
    /// Training jobs
    Training {
        #[command(subcommand)]
        cmd: TrainingCommand,
    },
    /// Model version registry
    Models {
        #[command(subcommand)]
        cmd: ModelsCommand,
    },
    /// Model deployments
    Deployments {
        #[command(subcommand)]
        cmd: DeploymentsCommand,
    },
    /// VLA skill library
    Skills {
        #[command(subcommand)]
        cmd: SkillsCommand,
    },
    /// Robot embodiment configs
    Embodiments {
        #[command(subcommand)]
        cmd: EmbodimentsCommand,
    },
    /// Model evaluation & metrics
    Eval {
        #[command(subcommand)]
        cmd: EvalCommand,
    },
}

// ── Robots ──────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum RobotsCommand {
    /// List all robots
    List,
    /// Show robot details
    Show { id: String },
    /// Register a robot by URL
    Register {
        /// Robot agent URL (e.g. http://192.168.178.45:41243)
        url: String,
    },
    /// Unregister a robot
    Remove { id: String },
    /// Show robot telemetry
    Telemetry {
        id: String,
        /// Show recent telemetry history instead of the latest frame
        #[arg(long)]
        history: bool,
    },
    /// Show agent-mode and E-stop state for a robot
    AgentMode { id: String },
    /// List a robot's cameras
    Cameras { id: String },
    /// List a robot's A2A peers
    Peers { id: String },
    /// Send a command to a robot
    Command {
        id: String,
        /// Command type
        #[arg(name = "type")]
        cmd_type: String,
        /// JSON payload
        #[arg(long)]
        payload: Option<String>,
        /// Command priority
        #[arg(long)]
        priority: Option<String>,
    },
}

// ── Alerts ──────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum AlertsCommand {
    /// List alerts
    List {
        /// Filter by severity (critical, error, warning, info)
        #[arg(long)]
        severity: Option<String>,
        /// Show only active (unacknowledged) alerts
        #[arg(long)]
        active: bool,
    },
    /// Show one alert
    Show { id: String },
    /// Acknowledge an alert
    Ack { id: String },
    /// Show alert history
    History,
    /// Delete one alert
    Delete { id: String },
    /// Bulk-delete alerts
    Clear {
        /// Delete only acknowledged alerts
        #[arg(long)]
        acknowledged: bool,
        /// Delete every alert
        #[arg(long)]
        all: bool,
    },
}

// ── Safety ──────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum SafetyCommand {
    /// Show E-stop and safety state (fleet-wide, or one robot)
    Status {
        /// Limit to a single robot
        #[arg(long)]
        robot: Option<String>,
    },
    /// Engage emergency stop
    Estop {
        /// Robot ID (omit with --fleet)
        id: Option<String>,
        /// Stop every robot in the fleet
        #[arg(long)]
        fleet: bool,
        /// Why the stop was triggered
        #[arg(long)]
        reason: Option<String>,
    },
    /// Reset a latched emergency stop
    Reset {
        /// Robot ID (omit with --fleet)
        id: Option<String>,
        /// Reset the whole fleet
        #[arg(long)]
        fleet: bool,
    },
    /// Engage emergency stop for every robot in a zone
    ZoneEstop {
        zone_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// List recent safety events
    Events,
}

// ── Incidents ───────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum IncidentsCommand {
    /// List incidents
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        severity: Option<String>,
        #[arg(long)]
        robot_id: Option<String>,
    },
    /// Show one incident
    Show { id: String },
    /// Incident dashboard rollup
    Dashboard,
    /// List open incidents
    Open,
    /// List overdue incidents
    Overdue,
    /// Severity/likelihood risk matrix
    RiskMatrix,
    /// List evidence attached to an incident
    Evidence { id: String },
}

// ── Oversight ───────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum OversightCommand {
    /// Oversight dashboard
    Dashboard,
    /// Fleet behaviour overview
    Fleet,
    /// List anomalies
    Anomalies {
        /// Only currently active anomalies
        #[arg(long)]
        active: bool,
        /// Only unacknowledged anomalies
        #[arg(long)]
        unacked: bool,
    },
    /// Acknowledge an anomaly
    AckAnomaly { id: String },
    /// Resolve an anomaly
    ResolveAnomaly {
        id: String,
        #[arg(long)]
        notes: Option<String>,
    },
    /// List manual-control sessions
    ManualSessions,
    /// Take or release manual control of a robot
    ManualMode {
        robot_id: String,
        #[arg(long)]
        reason: Option<String>,
        /// Release manual control instead of taking it
        #[arg(long)]
        off: bool,
    },
    /// List oversight verifications
    Verifications {
        /// Only verifications that are due
        #[arg(long)]
        due: bool,
    },
    /// Show a robot's oversight capabilities
    Capabilities { robot_id: String },
    /// Oversight audit log
    Logs,
}

// ── Approvals ───────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum ApprovalsCommand {
    /// List approval requests
    List,
    /// Show one approval request
    Show { id: String },
    /// Approvals waiting on you
    Pending {
        /// Show approvals pending for a role instead
        #[arg(long)]
        role: Option<String>,
    },
    /// Overdue approvals
    Overdue,
    /// Approvals nearing their deadline
    Nearing,
    /// Decide an approval step
    Decide {
        id: String,
        step_id: String,
        /// approve | reject | defer | request_info
        #[arg(long)]
        decision: String,
        #[arg(long)]
        notes: Option<String>,
    },
    /// Cancel an approval request
    Cancel { id: String },
    /// Escalate an approval request
    Escalate { id: String },
    /// Approval metrics
    Metrics,
    /// SLA report
    SlaReport,
}

// ── Teleoperation ───────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum TeleopCommand {
    /// List teleoperation sessions
    List {
        #[arg(long)]
        robot_id: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long = "type", name = "type")]
        session_type: Option<String>,
    },
    /// Show one session
    Show { id: String },
    /// List a session's episodes
    Episodes { id: String },
    /// Pause a session
    Pause { id: String },
    /// Resume a session
    Resume { id: String },
    /// End a session
    End { id: String },
}

// ── Patrol ──────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum PatrolCommand {
    /// List patrol routes
    Routes,
    /// Show one patrol route
    Route { id: String },
    /// List patrol runs
    Runs {
        #[arg(long)]
        route: Option<String>,
    },
    /// Show one patrol run
    Run { run_id: String },
    /// List patrol findings
    Findings,
    /// Show one finding
    Finding { id: String },
    /// List known places
    Places,
    /// Start a patrol
    Start {
        route_id: String,
        #[arg(long)]
        robot_id: Option<String>,
        /// baseline | patrol
        #[arg(long)]
        mode: Option<String>,
    },
    /// Abort a running patrol
    Abort {
        route_id: String,
        #[arg(long)]
        robot_id: Option<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Acknowledge a finding
    AckFinding { id: String },
    /// Mark a finding as normal (fold into the baseline)
    NormalFinding { id: String },
    /// Escalate a finding to an incident
    EscalateFinding { id: String },
    /// Promote a run to the baseline
    PromoteRun { run_id: String },
}

// ── Updates ─────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum UpdatesCommand {
    /// List available updates
    List,
    /// Show one update
    Show { id: String },
    /// Show update deployments for a robot
    Deployments { robot_id: String },
    /// Approve an update
    Approve {
        id: String,
        #[arg(long)]
        approver_id: String,
    },
    /// Deploy an update to a robot
    Deploy {
        id: String,
        robot_id: String,
        #[arg(long)]
        previous_version: Option<String>,
    },
    /// Roll a robot back to an earlier version
    Rollback {
        id: String,
        robot_id: String,
        #[arg(long)]
        target_version: String,
    },
}

// ── Processes ───────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum ProcessesCommand {
    /// List process definitions
    List,
    /// Show one process definition
    Show { id: String },
    /// List process instances
    Instances {
        #[arg(long)]
        status: Option<String>,
    },
    /// Show one process instance
    Instance { id: String },
    /// List tasks
    Tasks,
    /// Task queue statistics
    TaskQueue,
    /// Start a process definition
    Start { id: String },
    /// Pause a running instance
    Pause { instance_id: String },
    /// Resume a paused instance
    Resume { instance_id: String },
    /// Cancel an instance
    Cancel { instance_id: String },
    /// Retry a failed instance
    Retry { instance_id: String },
}

// ── Compliance ──────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum ComplianceCommand {
    /// Compliance score dashboard
    Dashboard,
    /// Upcoming and past-due deadlines
    Deadlines,
    /// Open compliance gaps
    Gaps {
        /// Show the rollup instead of individual gaps
        #[arg(long)]
        summary: bool,
    },
    /// Compliance-log metrics
    Metrics,
    /// Documents that are expiring
    Documents,
    /// Compliance training records
    Training {
        #[arg(long)]
        summary: bool,
    },
    /// Inspections
    Inspections {
        #[arg(long)]
        summary: bool,
    },
    /// Risk assessments
    RiskAssessments,
    /// Recent compliance activity
    Activity,
}

// ── Datasets ────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum DatasetsCommand {
    /// List datasets
    List,
    /// Show dataset details
    Show { id: String },
    /// List episodes in a dataset
    Episodes { id: String },
    /// Quality report
    Quality { id: String },
    /// Dataset statistics
    Stats { id: String },
    /// Validate a dataset
    Validate {
        id: String,
        /// Run the slower, deeper validation
        #[arg(long)]
        advanced: bool,
    },
    /// Push a dataset to the Hugging Face Hub
    Push {
        id: String,
        /// Hugging Face token
        #[arg(long)]
        hf_token: String,
        /// Target repo (e.g. org/dataset-name)
        #[arg(long)]
        repo_id: String,
    },
    /// Status of a running Hub push
    PushStatus { id: String },
    /// Upload a LeRobot dataset directory straight into RMS (no Hugging Face)
    ///
    /// Sends EVERY file of the directory — the LeRobot tree, README.md and
    /// whatever else is in it — file by file in bounded parts, verifies every
    /// part by checksum, waits for the server's validation and prints the
    /// dataset. Only .git, .DS_Store, __pycache__, a top-level .cache and
    /// *.rmsctl-part are left out, plus what --exclude names; run without
    /// --yes to see the list. Run the same command again to resume an
    /// interrupted upload: only the parts the server does not hold yet are
    /// sent. Before deleting the directory, run `datasets verify`.
    Upload {
        /// Dataset root: the directory that holds meta/ and data/
        dir: std::path::PathBuf,
        /// Leave files out: *.log, notes, raw/*.bag (repeatable)
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
        /// Upload only the LeRobot tree (meta/, data/**/*.parquet,
        /// videos/**/*.mp4, images/) — all that a server older than
        /// "stores every file" keeps
        #[arg(long)]
        lerobot_only: bool,
        /// Also upload .git, .DS_Store, __pycache__ and a top-level .cache
        #[arg(long)]
        no_default_excludes: bool,
        /// Dataset name (default: the directory name)
        #[arg(long)]
        name: Option<String>,
        /// Dataset description
        #[arg(long)]
        description: Option<String>,
        /// Robot type ID (default: matched from meta/info.json, created if new)
        #[arg(long, value_name = "ID")]
        robot_type: Option<String>,
        /// Part size: bytes, or e.g. 16M (min 5M; default and max set by the
        /// server, 32M). Lower it on a slow uplink: one part must arrive
        /// within the proxy's request timeout
        #[arg(long, value_name = "SIZE", value_parser = crate::commands::dataset_upload::parse_size)]
        chunk_size: Option<u64>,
        /// Parts sent at once (the server streams at most 4 across all uploads)
        #[arg(long, value_name = "N", default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=16))]
        parallel: u8,
        /// Attempts per part after a transient failure, with backoff
        #[arg(long, value_name = "N", default_value_t = 8)]
        retries: u32,
        /// Resume this upload instead of the one remembered for the directory
        #[arg(long, value_name = "DATASET_ID", conflicts_with = "restart")]
        resume: Option<String>,
        /// Discard the upload remembered for the directory and start over
        #[arg(long)]
        restart: bool,
        /// Skip the whole-file SHA-256 (parts are still verified one by one)
        #[arg(long)]
        no_checksum: bool,
        /// Return once every part is stored instead of waiting for validation
        #[arg(long)]
        no_wait: bool,
        /// Seconds to wait for the server's verdict after the last part
        #[arg(long, value_name = "SECS", default_value_t = 1800)]
        wait_timeout: u64,
        /// Seconds one part may take before it is sent again
        #[arg(long, value_name = "SECS", default_value_t = 300)]
        part_timeout: u64,
    },
    /// Download a dataset from RMS as the LeRobot directory it is stored as
    ///
    /// Reproduces every stored file — meta/, data/, videos/, README.md … —
    /// with the same relative paths and the same bytes, straight from the
    /// instance's storage (no Hugging Face).
    /// Every file is checked against the size the server lists, and against
    /// its SHA-256 where the server has one (datasets that were uploaded with
    /// rmsctl). Unfinished files end in .rmsctl-part; --resume continues them.
    Download {
        /// Dataset ID
        id: String,
        /// Directory to write the dataset into (default: ./<dataset name>)
        #[arg(long, short = 'o', value_name = "DIR")]
        out: Option<std::path::PathBuf>,
        /// Files fetched at once
        #[arg(long, value_name = "N", default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
        parallel: u8,
        /// Attempts per file after a transient failure, with backoff
        #[arg(long, value_name = "N", default_value_t = 8)]
        retries: u32,
        /// Continue in a non-empty directory: keep files that are complete,
        /// continue partial ones where they stopped
        #[arg(long, conflicts_with = "force")]
        resume: bool,
        /// Write into a non-empty directory and fetch every file again
        #[arg(long)]
        force: bool,
        /// Verify sizes only, even where the server lists a SHA-256
        #[arg(long)]
        no_checksum: bool,
        /// Seconds a request may deliver nothing before it is tried again
        #[arg(long, value_name = "SECS", default_value_t = 60)]
        stall_timeout: u64,
    },
    /// Check that every file of a local directory is on the instance
    ///
    /// The step before deleting a local dataset. Compares the directory with
    /// what the instance stores for the dataset: the same relative paths, the
    /// same sizes, and the same SHA-256 wherever the instance has one (a
    /// dataset uploaded with rmsctl). Only the file list is fetched — no file
    /// is downloaded; the local files are hashed. Reads the directory by the
    /// rules `datasets upload` does, so pass the same --exclude. Lists every
    /// difference and exits non-zero when there is one.
    Verify {
        /// Dataset ID
        id: String,
        /// The local dataset directory to compare
        dir: std::path::PathBuf,
        /// Leave files out of the comparison, as on upload (repeatable)
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
        /// Compare only the LeRobot tree, as `upload --lerobot-only` sent it
        #[arg(long)]
        lerobot_only: bool,
        /// Also compare .git, .DS_Store, __pycache__ and a top-level .cache
        #[arg(long)]
        no_default_excludes: bool,
        /// Compare sizes only, even where the instance lists a SHA-256
        #[arg(long, conflicts_with = "require_checksum")]
        no_checksum: bool,
        /// Fail for any file the instance lists no SHA-256 for
        #[arg(long)]
        require_checksum: bool,
        /// Files hashed at once
        #[arg(long, value_name = "N", default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
        parallel: u8,
    },
    /// What the server holds of a direct upload
    UploadStatus { id: String },
    /// Cancel a direct upload and delete what was stored
    UploadCancel { id: String },
    /// List flagged episodes
    Flagged { id: String },
    /// Flag one episode
    Flag {
        id: String,
        index: u32,
        #[arg(long)]
        reason: Option<String>,
    },
}

// ── Training ────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum TrainingCommand {
    /// List training jobs
    List,
    /// Show training job details
    Show { id: String },
    /// Show currently running jobs
    Active,
    /// Show queue statistics
    Queue,
    /// List training workers
    Workers,
    /// Start a training job from a JSON config file
    Start {
        /// Path to a JSON file holding the job config
        #[arg(long)]
        config: String,
    },
    /// Cancel a training job
    Cancel { id: String },
    /// Retry a failed training job
    Retry { id: String },
    /// Show a job's export manifest
    Export { id: String },
    /// Show a job's runtime estimate
    Estimate { id: String },
}

// ── Models ──────────────────────────────────────────────────────────────

// Parsed once per run and matched at once: the size of `Upload` costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum ModelsCommand {
    /// List model versions
    List,
    /// Show a model version and its evaluation rollup
    Show { id: String },
    /// Show a model version's lineage
    Lineage { id: String },
    /// Compare the metrics of two model versions
    Compare {
        version_a: String,
        version_b: String,
        /// Metric keys to diff (comma-separated); omit for all
        #[arg(long)]
        metrics: Option<String>,
    },
    /// Upload a trained checkpoint directory straight into RMS and register it
    ///
    /// Sends every file of the directory in bounded parts, verifies every part
    /// by checksum, and waits until the server has registered the model. Run
    /// the same command again to resume an interrupted upload: only the parts
    /// the server does not hold yet are sent.
    Upload {
        /// Checkpoint directory: the one holding config.json and the weights
        dir: std::path::PathBuf,
        /// Model name (default: the directory name)
        #[arg(long)]
        name: Option<String>,
        /// Version of this model, e.g. 1.0.0 or step-5000
        #[arg(long = "version", alias = "model-version", value_name = "VERSION")]
        model_version: String,
        /// Kind of model: vla (default) or rl_policy
        #[arg(long, value_name = "TYPE")]
        model_type: Option<String>,
        /// The foundation model it was fine-tuned from: pi0, pi0_6, openvla,
        /// groot, groot_n1_7 or smolvla
        #[arg(long, value_name = "MODEL")]
        base_model: Option<String>,
        /// ID of the dataset it was trained on (must exist on the instance)
        #[arg(long, value_name = "ID")]
        dataset: Option<String>,
        /// Training metrics: key=value, or a JSON file holding an object (repeatable)
        #[arg(long, value_name = "K=V|FILE")]
        metrics: Vec<String>,
        /// Evaluation metrics, same forms, e.g. success_rate=0.52 (repeatable)
        #[arg(long, value_name = "K=V|FILE")]
        eval_metrics: Vec<String>,
        /// Free text shown with the model: recipe, seed, where it was evaluated
        #[arg(long)]
        notes: Option<String>,
        /// Leave files out: *.log, wandb, checkpoint-*/optimizer.pt (repeatable)
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
        /// Part size: bytes, or e.g. 16M (min 5M; default and max set by the
        /// server, 32M). Lower it on a slow uplink
        #[arg(long, value_name = "SIZE", value_parser = crate::commands::dataset_upload::parse_size)]
        chunk_size: Option<u64>,
        /// Parts sent at once (the server streams at most 4 across all uploads)
        #[arg(long, value_name = "N", default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=16))]
        parallel: u8,
        /// Attempts per part after a transient failure, with backoff
        #[arg(long, value_name = "N", default_value_t = 8)]
        retries: u32,
        /// Resume this upload instead of the one remembered for the directory
        #[arg(long, value_name = "UPLOAD_ID", conflicts_with = "restart")]
        resume: Option<String>,
        /// Discard the upload remembered for the directory and start over
        #[arg(long)]
        restart: bool,
        /// Skip the whole-file SHA-256 (parts are still verified one by one)
        #[arg(long)]
        no_checksum: bool,
        /// Return once every part is stored instead of waiting for the registration
        #[arg(long)]
        no_wait: bool,
        /// Seconds to wait for the server to assemble, verify and register
        #[arg(long, value_name = "SECS", default_value_t = 3600)]
        wait_timeout: u64,
        /// Seconds one part may take before it is sent again
        #[arg(long, value_name = "SECS", default_value_t = 300)]
        part_timeout: u64,
    },
    /// What the server holds of a model upload; without an ID, every open upload
    UploadStatus { id: Option<String> },
    /// Cancel a model upload and delete what was stored
    UploadCancel { id: String },
    /// List the files the instance stores for a model
    Files { id: String },
    /// Download an uploaded model's files as the directory they were stored as
    ///
    /// Same relative paths, same bytes, straight from the instance's storage.
    /// Every file is checked against the size the server lists, and against
    /// its SHA-256 where the upload declared one (rmsctl does). Unfinished
    /// files end in .rmsctl-part; --resume continues them.
    Download {
        /// Model version ID
        id: String,
        /// Directory to write the model into (default: ./<name>-<version>)
        #[arg(long, short = 'o', value_name = "DIR")]
        out: Option<std::path::PathBuf>,
        /// Files fetched at once
        #[arg(long, value_name = "N", default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
        parallel: u8,
        /// Attempts per file after a transient failure, with backoff
        #[arg(long, value_name = "N", default_value_t = 8)]
        retries: u32,
        /// Continue in a non-empty directory: keep files that are complete,
        /// continue partial ones where they stopped
        #[arg(long, conflicts_with = "force")]
        resume: bool,
        /// Write into a non-empty directory and fetch every file again
        #[arg(long)]
        force: bool,
        /// Verify sizes only, even where the server lists a SHA-256
        #[arg(long)]
        no_checksum: bool,
        /// Seconds a request may deliver nothing before it is tried again
        #[arg(long, value_name = "SECS", default_value_t = 60)]
        stall_timeout: u64,
    },
    /// Archive a model version; a model uploaded to the instance also has its
    /// stored files removed
    Delete { id: String },
}

// ── Deployments ─────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum DeploymentsCommand {
    /// List deployments
    List {
        /// Only in-flight deployments
        #[arg(long)]
        active: bool,
    },
    /// Show deployment details
    Show { id: String },
    /// Show deployment metrics
    Metrics { id: String },
    /// Promote a deployment to the next stage
    Promote { id: String },
    /// Roll a deployment back
    Rollback {
        id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Cancel a deployment
    Cancel { id: String },
}

// ── Skills ──────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List skills
    List {
        /// Only published skills
        #[arg(long)]
        published: bool,
    },
    /// Show skill details
    Show { id: String },
    /// List skills available for a robot
    ForRobot { robot_id: String },
    /// List robots compatible with a skill
    Compatible { id: String },
    /// Check one skill against one robot
    Check { id: String, robot_id: String },
    /// Validate a skill definition
    Validate { id: String },
    /// Publish a skill
    Publish { id: String },
    /// Deprecate a skill
    Deprecate { id: String },
    /// Archive a skill
    Archive { id: String },
    /// Execute a skill on a robot
    Execute {
        id: String,
        #[arg(long)]
        robot_id: String,
        /// Task prompt passed to the policy
        #[arg(long)]
        prompt: Option<String>,
    },
    /// Abort a running skill execution
    Abort { id: String },
}

// ── Embodiments ─────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum EmbodimentsCommand {
    /// List embodiment configs
    List,
    /// Show embodiment details
    Show { tag: String },
}

// ── Eval ────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum EvalCommand {
    /// List evaluation episodes
    Episodes {
        #[arg(long)]
        robot_id: Option<String>,
        #[arg(long)]
        model_version: Option<String>,
    },
    /// Show success rate
    SuccessRate {
        #[arg(long)]
        robot_id: Option<String>,
        #[arg(long)]
        model_version: Option<String>,
        /// Time window (24h, 7d, 30d, all)
        #[arg(long)]
        period: Option<String>,
    },
    /// Show one evaluation episode
    Episode { id: String },
    /// Error breakdown
    Errors {
        #[arg(long)]
        robot_id: Option<String>,
        #[arg(long)]
        model_version: Option<String>,
    },
    /// Reward-model scores
    Rewards,
    /// Compare two model versions
    Compare {
        version_a: String,
        version_b: String,
        /// Time window (24h, 7d, 30d, all)
        #[arg(long)]
        period: Option<String>,
    },
    /// Trigger a hardware evaluation run
    RunHardware {
        #[arg(long)]
        robot_id: String,
        #[arg(long)]
        skill_id: String,
        #[arg(long)]
        episodes: Option<u32>,
        #[arg(long)]
        prompt: Option<String>,
    },
}
