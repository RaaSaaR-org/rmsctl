//! Types shared by more than one command module.
//!
//! Anything used by a single module lives in that module instead — the server's
//! envelopes are inconsistent enough (`{robots}`, `{data}`, `{sessions}`, bare
//! arrays, flat objects) that a shared struct per endpoint was doing more harm
//! than good.

use serde::{Deserialize, Serialize};

/// Pagination envelope.
///
/// Every field is optional and defaulted because the server does not agree with
/// itself: most routes send `pageSize`, `/datasets` and the history routes send
/// `limit`, and a few omit `totalPages`.
#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default, alias = "limit")]
    pub page_size: Option<u32>,
    #[serde(default)]
    pub total: Option<u32>,
    #[serde(default)]
    pub total_pages: Option<u32>,
}

impl Pagination {
    pub fn total(&self) -> Option<u32> {
        self.total
    }
    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

// ── Robots ──────────────────────────────────────────────────────────────
// Used by `robots` and by the `status` dashboard.

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
    pub capabilities: Option<serde_json::Value>,
    /// The agent URL. `baseUrl` is what POST /robots/register returns; the
    /// list and detail routes call the same thing `a2aAgentUrl`.
    #[serde(default, alias = "baseUrl")]
    pub a2a_agent_url: Option<String>,
    #[serde(default)]
    pub a2a_enabled: Option<bool>,
    #[serde(default)]
    pub current_task_id: Option<String>,
    #[serde(default)]
    pub current_task_name: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl Robot {
    pub fn zone(&self) -> Option<&str> {
        self.location.as_ref()?.get("zone")?.as_str()
    }
    pub fn floor(&self) -> Option<&str> {
        self.location.as_ref()?.get("floor")?.as_str()
    }
    /// What to call a robot that is on the fleet but has never connected, or
    /// `None` for every other robot.
    ///
    /// The server lists an expected robot (`PUT /robots/:id/expected`) as
    /// `offline` with `metadata.provisioning` = `awaiting_agent` or `arriving`.
    /// The web UI reads those as "Not connected" and "Arriving", never as
    /// "Offline"; so does this. A robot that is not offline is connected,
    /// whatever stale metadata says.
    pub fn expected_label(&self) -> Option<&'static str> {
        if self.status.as_deref() != Some("offline") {
            return None;
        }
        match self.metadata.as_ref()?.get("provisioning")?.as_str()? {
            "awaiting_agent" => Some("not connected"),
            "arriving" => Some("arriving"),
            _ => None,
        }
    }
    /// How a robot's state should be bucketed on the fleet dashboard.
    pub fn health(&self) -> RobotHealth {
        if self.expected_label().is_some() {
            return RobotHealth::Expected;
        }
        match self
            .status
            .as_deref()
            .unwrap_or("unknown")
            .to_lowercase()
            .as_str()
        {
            "emergency_stop" | "estop" | "e_stop" | "protective_stop" | "safety_stop" => {
                RobotHealth::Stopped
            }
            "online" | "active" | "connected" | "idle" | "ready" => RobotHealth::Online,
            "busy" | "running" | "charging" | "moving" | "executing" | "manual" | "teleop" => {
                RobotHealth::Working
            }
            "error" | "failed" | "degraded" => RobotHealth::Error,
            "offline" | "disconnected" => RobotHealth::Offline,
            _ => RobotHealth::Other,
        }
    }
}

/// Buckets for the fleet overview.
///
/// `Stopped` and `Working` exist because the old dashboard folded every state
/// it did not recognise into "Other" and then counted availability as
/// `online / total` — a fleet with a latched E-stop read as merely idle.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RobotHealth {
    Online,
    Working,
    Stopped,
    Error,
    Offline,
    /// On the fleet, never connected yet — not an outage, and not part of the
    /// fleet availability is measured over.
    Expected,
    Other,
}

impl RobotHealth {
    /// Counts toward availability: reachable and not latched.
    pub fn is_available(&self) -> bool {
        matches!(self, RobotHealth::Online | RobotHealth::Working)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RobotsResponse {
    pub robots: Vec<Robot>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

// ── Alerts ──────────────────────────────────────────────────────────────
// Used by `alerts` and by the `status` dashboard.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub id: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub acknowledged: Option<bool>,
    #[serde(default)]
    pub acknowledged_by: Option<String>,
    /// The server names this `timestamp`; `createdAt` is accepted so older
    /// payloads still read.
    #[serde(default, alias = "createdAt")]
    pub timestamp: Option<String>,
}

/// `GET /api/alerts` wraps its rows in `data`, not `alerts`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertsResponse {
    #[serde(default, alias = "alerts")]
    pub data: Vec<Alert>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
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

impl AlertCounts {
    pub fn total(&self) -> u32 {
        self.critical + self.error + self.warning + self.info
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertCountsResponse {
    #[serde(default)]
    pub counts: AlertCounts,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn robot_with(status: &str) -> Robot {
        serde_json::from_value(serde_json::json!({ "id": "r1", "status": status })).unwrap()
    }

    fn expected_robot(status: &str, provisioning: &str) -> Robot {
        serde_json::from_value(serde_json::json!({
            "id": "r1",
            "status": status,
            "metadata": { "provisioning": provisioning, "plannedPlace": "STAGING" }
        }))
        .unwrap()
    }

    #[test]
    fn an_expected_robot_is_not_offline() {
        let waiting = expected_robot("offline", "awaiting_agent");
        assert_eq!(waiting.expected_label(), Some("not connected"));
        assert_eq!(waiting.health(), RobotHealth::Expected);
        assert!(!waiting.health().is_available());

        let arriving = expected_robot("offline", "arriving");
        assert_eq!(arriving.expected_label(), Some("arriving"));
        assert_eq!(arriving.health(), RobotHealth::Expected);
    }

    #[test]
    fn only_an_offline_robot_with_a_known_provisioning_state_is_expected() {
        // Connected, with the metadata of its expected days still on the row.
        let online = expected_robot("online", "awaiting_agent");
        assert_eq!(online.expected_label(), None);
        assert_eq!(online.health(), RobotHealth::Online);
        // A state this build does not know stays what the server says: offline.
        let unknown = expected_robot("offline", "decommissioned");
        assert_eq!(unknown.expected_label(), None);
        assert_eq!(unknown.health(), RobotHealth::Offline);
        // No metadata at all, or a non-string value.
        assert_eq!(robot_with("offline").expected_label(), None);
        let odd: Robot = serde_json::from_value(serde_json::json!({
            "id": "r1", "status": "offline", "metadata": { "provisioning": true }
        }))
        .unwrap();
        assert_eq!(odd.health(), RobotHealth::Offline);
    }

    /// The fleet dashboard folded every status it did not recognise into
    /// "Other" and computed availability as `online / total`, so a robot held
    /// in protective stop read as merely idle.
    #[test]
    fn safety_stops_are_never_available() {
        for s in [
            "emergency_stop",
            "estop",
            "e_stop",
            "protective_stop",
            "safety_stop",
        ] {
            let h = robot_with(s).health();
            assert_eq!(h, RobotHealth::Stopped, "{s} should bucket as Stopped");
            assert!(!h.is_available(), "{s} must not count as available");
        }
    }

    #[test]
    fn working_states_are_available_but_distinct_from_idle() {
        for s in ["busy", "running", "charging", "moving", "teleop"] {
            let h = robot_with(s).health();
            assert_eq!(h, RobotHealth::Working, "{s} should bucket as Working");
            assert!(h.is_available());
        }
        assert_eq!(robot_with("online").health(), RobotHealth::Online);
        assert!(robot_with("online").health().is_available());
    }

    #[test]
    fn faults_and_absence_are_not_available() {
        assert_eq!(robot_with("error").health(), RobotHealth::Error);
        assert_eq!(robot_with("offline").health(), RobotHealth::Offline);
        assert!(!robot_with("error").health().is_available());
        assert!(!robot_with("offline").health().is_available());
        assert!(!robot_with("wat").health().is_available());
    }

    #[test]
    fn status_matching_is_case_insensitive() {
        assert_eq!(robot_with("PROTECTIVE_STOP").health(), RobotHealth::Stopped);
        assert_eq!(robot_with("Online").health(), RobotHealth::Online);
    }

    /// A missing field must never be a hard parse failure: the server drops
    /// keys freely and the CLI has to keep rendering.
    #[test]
    fn a_robot_needs_only_an_id() {
        let r: Robot = serde_json::from_value(serde_json::json!({ "id": "r1" })).unwrap();
        assert_eq!(r.health(), RobotHealth::Other);
        assert_eq!(r.zone(), None);
        assert_eq!(r.name, None);
    }

    /// `/datasets` names the page size `limit`; everything else says `pageSize`.
    #[test]
    fn pagination_accepts_both_page_size_spellings() {
        let a: Pagination =
            serde_json::from_value(serde_json::json!({"page":1,"pageSize":50,"total":75})).unwrap();
        let b: Pagination =
            serde_json::from_value(serde_json::json!({"page":1,"limit":20,"total":14})).unwrap();
        assert_eq!(a.page_size, Some(50));
        assert_eq!(b.page_size, Some(20));
        assert_eq!(a.total(), Some(75));
    }

    /// `/alerts` returns its rows under `data`, not `alerts`.
    #[test]
    fn alerts_envelope_reads_data_and_timestamp() {
        let r: AlertsResponse = serde_json::from_value(serde_json::json!({
            "data": [{"id":"a1","severity":"warning","title":"x","timestamp":"2026-01-01T00:00:00Z"}],
            "pagination": {"page":1,"pageSize":50,"total":75,"totalPages":2}
        }))
        .unwrap();
        assert_eq!(r.data.len(), 1);
        assert_eq!(r.data[0].timestamp.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(r.pagination.unwrap().total_pages(), Some(2));
    }

    #[test]
    fn alert_counts_total_adds_every_bucket() {
        let c: AlertCounts = serde_json::from_value(
            serde_json::json!({"critical":1,"error":2,"warning":3,"info":4}),
        )
        .unwrap();
        assert_eq!(c.total(), 10);
        assert_eq!(AlertCounts::default().total(), 0);
    }
}
