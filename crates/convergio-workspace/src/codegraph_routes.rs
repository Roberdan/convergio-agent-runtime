//! HTTP routes for the code graph API.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::codegraph;
use crate::routes::WorkspaceState;

pub fn codegraph_routes(state: Arc<WorkspaceState>) -> Router {
    Router::new()
        .route("/api/codegraph/expand", post(handle_expand))
        .route("/api/codegraph/package-deps", get(handle_package_deps))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct ExpandRequest {
    files: Vec<String>,
}

/// POST /api/codegraph/expand — expand file list through dependency graph.
async fn handle_expand(
    State(state): State<Arc<WorkspaceState>>,
    Json(req): Json<ExpandRequest>,
) -> Json<Value> {
    // Input validation: reject traversal and limit file count
    if req.files.len() > 50 {
        return Json(json!({"error": "too many files (max 50)"}));
    }
    for f in &req.files {
        if f.contains("..") {
            return Json(json!({"error": "path traversal not allowed"}));
        }
    }
    let root = std::path::Path::new(&state.repo_root);
    let result = codegraph::expand_files(&req.files, root);
    Json(json!(result))
}

/// GET /api/codegraph/package-deps — return package-level dependency map.
async fn handle_package_deps(State(state): State<Arc<WorkspaceState>>) -> Json<Value> {
    let root = std::path::Path::new(&state.repo_root);
    let deps = codegraph::package_deps(root);
    Json(json!(deps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codegraph_routes_builds() {
        let state = Arc::new(WorkspaceState {
            repo_root: ".".to_string(),
        });
        let _router = codegraph_routes(state);
    }
}
