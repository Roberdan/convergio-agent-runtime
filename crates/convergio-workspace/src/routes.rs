//! HTTP routes for workspace management.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde_json::{json, Value};

use crate::guard::{handle_check_owner, handle_list, handle_list_owned};
use crate::reaper::{reap_cycle, ReapReport, STALE_THRESHOLD};

pub struct WorkspaceState {
    pub repo_root: String,
}

pub fn workspace_routes(state: Arc<WorkspaceState>) -> Router {
    Router::new()
        .route("/api/workspace/reap", post(handle_reap))
        .route("/api/workspace/gc", post(handle_gc))
        .route("/api/workspace/check-owner", post(handle_check_owner))
        .route("/api/workspace/list", get(handle_list))
        .route("/api/workspace/list-owned", get(handle_list_owned))
        .with_state(state)
}

async fn handle_reap(State(state): State<Arc<WorkspaceState>>) -> Json<Value> {
    let repo_root = std::path::Path::new(&state.repo_root);
    let report: ReapReport = reap_cycle(repo_root, STALE_THRESHOLD);
    Json(json!({
        "ok": true,
        "reaped": report.reaped,
        "branches_deleted": report.branches_deleted,
        "errors": report.errors,
        "skipped": report.skipped,
    }))
}

#[derive(serde::Deserialize)]
struct GcRequest {
    /// Threshold in minutes. Default: 60 (1 hour). Set to 0 for aggressive cleanup.
    #[serde(default = "default_gc_threshold")]
    threshold_minutes: u64,
}

fn default_gc_threshold() -> u64 {
    60
}

/// POST /api/workspace/gc — aggressive garbage collection with configurable threshold.
async fn handle_gc(
    State(state): State<Arc<WorkspaceState>>,
    Json(req): Json<GcRequest>,
) -> Json<Value> {
    let repo_root = std::path::Path::new(&state.repo_root);
    let threshold = std::time::Duration::from_secs(req.threshold_minutes.saturating_mul(60));
    let report = reap_cycle(repo_root, threshold);
    Json(json!({
        "ok": true,
        "threshold_minutes": req.threshold_minutes,
        "reaped": report.reaped,
        "branches_deleted": report.branches_deleted,
        "errors": report.errors,
        "skipped": report.skipped,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_routes_builds() {
        let state = Arc::new(WorkspaceState {
            repo_root: ".".to_string(),
        });
        let _router = workspace_routes(state);
    }
}
