use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rmsctl", about = "NeoDEM robot management CLI", version)]
pub struct Cli {
    /// Server URL (default: http://localhost:3001/api)
    #[arg(long, env = "RMS_SERVER_URL")]
    pub server: Option<String>,

    /// JWT auth token
    #[arg(long, env = "RMS_TOKEN")]
    pub token: Option<String>,

    /// Output as JSON instead of formatted text
    #[arg(long)]
    pub json: bool,

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
    /// Browse datasets
    Datasets {
        #[command(subcommand)]
        cmd: DatasetsCommand,
    },
    /// Browse training jobs
    Training {
        #[command(subcommand)]
        cmd: TrainingCommand,
    },
    /// Browse model registry
    Models {
        #[command(subcommand)]
        cmd: ModelsCommand,
    },
    /// Browse deployments
    Deployments {
        #[command(subcommand)]
        cmd: DeploymentsCommand,
    },
    /// Browse VLA skill library
    Skills {
        #[command(subcommand)]
        cmd: SkillsCommand,
    },
    /// Browse robot embodiment configs
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

#[derive(Subcommand)]
pub enum RobotsCommand {
    /// List all robots
    List,
    /// Show robot details
    Show {
        /// Robot ID
        id: String,
    },
    /// Register a robot by URL
    Register {
        /// Robot agent URL (e.g. http://192.168.178.45:41243)
        url: String,
    },
    /// Unregister a robot
    Remove {
        /// Robot ID
        id: String,
    },
    /// Show robot telemetry
    Telemetry {
        /// Robot ID
        id: String,
    },
    /// Send a command to a robot
    Command {
        /// Robot ID
        id: String,
        /// Command type
        #[arg(name = "type")]
        cmd_type: String,
        /// JSON payload
        #[arg(long)]
        payload: Option<String>,
    },
}

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
    /// Acknowledge an alert
    Ack {
        /// Alert ID
        id: String,
    },
}

#[derive(Subcommand)]
pub enum DatasetsCommand {
    /// List datasets
    List,
    /// Show dataset details
    Show { id: String },
    /// List episodes in a dataset
    Episodes { id: String },
}

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
}

#[derive(Subcommand)]
pub enum ModelsCommand {
    /// List registered models
    List,
    /// Show model details and versions
    Show { name: String },
    /// Compare two training runs
    Compare {
        /// First run ID
        run_a: String,
        /// Second run ID
        run_b: String,
        /// Metric keys (comma-separated)
        #[arg(long, default_value = "loss")]
        metrics: String,
    },
}

#[derive(Subcommand)]
pub enum DeploymentsCommand {
    /// List deployments
    List,
    /// Show deployment details
    Show { id: String },
}

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List skills
    List,
    /// Show skill details
    Show { id: String },
}

#[derive(Subcommand)]
pub enum EmbodimentsCommand {
    /// List embodiment configs
    List,
    /// Show embodiment details
    Show { tag: String },
}

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
    },
    /// Compare two model versions
    Compare {
        version_a: String,
        version_b: String,
    },
}
