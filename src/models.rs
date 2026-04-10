use serde::{Deserialize, Serialize};

// ── Pagination ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    pub page: u32,
    #[serde(default)]
    pub page_size: u32,
    pub total: u32,
    #[serde(default)]
    pub total_pages: u32,
}

// ── Robots ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Robot {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub battery_level: Option<f64>,
    #[serde(default)]
    pub location: Option<serde_json::Value>,
    #[serde(default)]
    pub last_seen: Option<String>,
    #[serde(default)]
    pub ip_address: Option<String>,
    #[serde(default)]
    pub firmware: Option<String>,
    #[serde(default)]
    pub is_connected: Option<bool>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub capabilities: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RobotsResponse {
    pub robots: Vec<Robot>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub robot_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RegisterResponse {
    pub robot: Robot,
    #[serde(default)]
    pub endpoints: Option<serde_json::Value>,
    #[serde(default)]
    pub agent_card: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct CommandRequest {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeleteResponse {
    pub success: bool,
}

// ── Alerts ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub id: String,
    pub severity: String,
    pub title: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub acknowledged: Option<bool>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AlertsResponse {
    #[serde(alias = "data")]
    pub alerts: Vec<Alert>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AlertCounts {
    #[serde(default)]
    pub critical: u32,
    #[serde(default)]
    pub error: u32,
    #[serde(default)]
    pub warning: u32,
    #[serde(default)]
    pub info: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AlertCountsResponse {
    pub counts: AlertCounts,
}

// ── Datasets ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dataset {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub demonstration_count: Option<u32>,
    #[serde(default)]
    pub total_frames: Option<u64>,
    #[serde(default)]
    pub fps: Option<f64>,
    #[serde(default)]
    pub robot_type_id: Option<String>,
    #[serde(default)]
    pub skill_id: Option<String>,
    #[serde(default)]
    pub quality_score: Option<f64>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DatasetsResponse {
    pub datasets: Vec<Dataset>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetEpisode {
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub frame_count: Option<u32>,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    #[serde(default)]
    pub flagged: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DatasetEpisodesResponse {
    pub episodes: Vec<DatasetEpisode>,
}

// ── Training ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingJob {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub base_model: Option<String>,
    #[serde(default)]
    pub fine_tune_method: Option<String>,
    #[serde(default)]
    pub dataset_id: Option<String>,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub current_epoch: Option<u32>,
    #[serde(default)]
    pub total_epochs: Option<u32>,
    #[serde(default)]
    pub current_loss: Option<f64>,
    #[serde(default)]
    pub learning_rate: Option<f64>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrainingJobsResponse {
    pub jobs: Vec<TrainingJob>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrainingActiveResponse {
    pub jobs: Vec<TrainingJob>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrainingJobDetailResponse {
    pub job: TrainingJob,
    #[serde(default)]
    pub progress: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct QueueStats {
    #[serde(default)]
    pub pending: Option<u32>,
    #[serde(default)]
    pub active: Option<u32>,
    #[serde(default)]
    pub completed: Option<u32>,
    #[serde(default)]
    pub failed: Option<u32>,
}

// ── Models ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredModel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub creation_timestamp: Option<serde_json::Value>,
    #[serde(default)]
    pub last_updated_timestamp: Option<serde_json::Value>,
    #[serde(default, alias = "latestVersions")]
    pub latest_versions: Option<Vec<ModelVersion>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelVersion {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub current_stage: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub creation_timestamp: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryResponse {
    pub registered_models: Vec<RegisteredModel>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDetailResponse {
    pub registered_model: RegisteredModel,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelVersionsResponse {
    #[serde(default)]
    pub model_name: Option<String>,
    pub versions: Vec<ModelVersion>,
    #[serde(default)]
    pub count: Option<u32>,
}

// ── Deployments ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub model_version_id: Option<String>,
    #[serde(default)]
    pub current_stage: Option<u32>,
    #[serde(default)]
    pub total_stages: Option<u32>,
    #[serde(default)]
    pub eligible_robot_count: Option<u32>,
    #[serde(default)]
    pub deployed_count: Option<u32>,
    #[serde(default)]
    pub failed_count: Option<u32>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeploymentsResponse {
    pub deployments: Vec<Deployment>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentDetailResponse {
    pub deployment: Deployment,
    #[serde(default)]
    pub model_version: Option<serde_json::Value>,
    #[serde(default)]
    pub metrics: Option<serde_json::Value>,
}

// ── Skills ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub robot_type_id: Option<String>,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    #[serde(default)]
    pub linked_model_version_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SkillsResponse {
    pub skills: Vec<Skill>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SkillDetailResponse {
    pub skill: Skill,
}

// ── Embodiments ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Embodiment {
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub action_dim: Option<u32>,
    #[serde(default)]
    pub proprioception_dim: Option<u32>,
    #[serde(default)]
    pub robot_type_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EmbodimentsResponse {
    pub embodiments: Vec<Embodiment>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EmbodimentDetailResponse {
    pub embodiment: Embodiment,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EmbodimentConfigResponse {
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
}

// ── Evaluation ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalEpisode {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub robot_id: Option<String>,
    #[serde(default)]
    pub model_version: Option<String>,
    #[serde(default)]
    pub task_prompt: Option<String>,
    #[serde(default)]
    pub success: Option<bool>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub started_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EvalEpisodesResponse {
    pub episodes: Vec<EvalEpisode>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessRate {
    #[serde(default)]
    pub model_version: Option<String>,
    #[serde(default)]
    pub success_rate: Option<f64>,
    #[serde(default)]
    pub total_episodes: Option<u32>,
    #[serde(default)]
    pub success_count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalComparison {
    #[serde(default)]
    pub version_a: Option<String>,
    #[serde(default)]
    pub version_b: Option<String>,
    #[serde(default)]
    pub success_rate_a: Option<f64>,
    #[serde(default)]
    pub success_rate_b: Option<f64>,
    #[serde(default)]
    pub improvement: Option<f64>,
}
