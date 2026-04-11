//! Worktree ownership guard — API endpoints for checking/listing worktree owners.
//!
//! Prevents agents from accidentally destroying each other's worktrees by
//! providing ownership verification before any destructive operation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::routes::WorkspaceState;

/// Request body for `POST /api/workspace/check-owner`.
#[derive(Debug, Deserialize)]
pub struct CheckOwnerRequest {
    pub worktree_path: String,
    pub agent_id: String,
}

/// Response for `POST /api/workspace/check-owner`.
#[derive(Debug, Serialize)]
pub struct CheckOwnerResponse {
    pub owned: bool,
    pub owner: Option<String>,
    pub task: Option<String>,
    pub created_at: Option<String>,
}

/// Query params for `GET /api/workspace/list-owned`.
#[derive(Debug, Deserialize)]
pub struct ListOwnedQuery {
    pub agent_id: String,
}

/// Query params for `GET /api/workspace/list`.
#[derive(Debug, Deserialize)]
pub struct ListWorkspacesQuery {
    pub plan_id: Option<i64>,
}

/// One entry in the list-owned response.
#[derive(Debug, Serialize)]
pub struct OwnedWorktree {
    pub path: String,
    pub task: Option<String>,
    pub created_at: Option<String>,
}

/// Response for `GET /api/workspace/list-owned`.
#[derive(Debug, Serialize)]
pub struct ListOwnedResponse {
    pub agent_id: String,
    pub worktrees: Vec<OwnedWorktree>,
}

/// One entry in the generic workspace list response.
#[derive(Debug, Serialize)]
pub struct WorkspaceEntry {
    pub workspace_id: String,
    pub path: String,
    pub status: String,
    pub wave_db_id: Option<i64>,
    pub owner: Option<String>,
    pub task: Option<String>,
}

/// Response for `GET /api/workspace/list`.
#[derive(Debug, Serialize)]
pub struct ListWorkspacesResponse {
    pub plan_id: Option<i64>,
    pub workspaces: Vec<WorkspaceEntry>,
}

/// Read and parse `.worktree-owner` from a directory.
fn read_owner_file(dir: &Path) -> Option<Value> {
    let path = dir.join(".worktree-owner");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Resolve worktree path — if relative, resolve against repo_root/.worktrees/.
/// Validates against path traversal before returning.
fn resolve_worktree_path(repo_root: &str, raw: &str) -> Result<PathBuf, String> {
    // Reject traversal components in raw input
    if raw.contains("..") {
        return Err("path traversal detected".into());
    }
    let p = Path::new(raw);
    let resolved = if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(repo_root).join(".worktrees").join(raw)
    };
    // Ensure resolved path stays within repo_root
    let base = Path::new(repo_root)
        .canonicalize()
        .map_err(|e| format!("invalid repo_root: {e}"))?;
    let canonical = resolved
        .canonicalize()
        .map_err(|e| format!("invalid path: {e}"))?;
    if !canonical.starts_with(&base) {
        return Err("path is outside repository".into());
    }
    Ok(canonical)
}

/// Handler: check if a specific agent owns a worktree.
pub async fn handle_check_owner(
    State(state): State<Arc<WorkspaceState>>,
    Json(req): Json<CheckOwnerRequest>,
) -> Json<CheckOwnerResponse> {
    let wt_path = match resolve_worktree_path(&state.repo_root, &req.worktree_path) {
        Ok(p) => p,
        Err(_) => {
            return Json(CheckOwnerResponse {
                owned: false,
                owner: None,
                task: None,
                created_at: None,
            })
        }
    };
    let owner_data = read_owner_file(&wt_path);

    match owner_data {
        Some(data) => {
            let file_agent = data.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
            Json(CheckOwnerResponse {
                owned: file_agent == req.agent_id,
                owner: Some(file_agent.to_string()),
                task: data.get("task").and_then(|v| v.as_str()).map(String::from),
                created_at: data
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        }
        None => Json(CheckOwnerResponse {
            owned: false,
            owner: None,
            task: None,
            created_at: None,
        }),
    }
}

/// Handler: list all worktrees owned by a given agent.
pub async fn handle_list_owned(
    State(state): State<Arc<WorkspaceState>>,
    Query(query): Query<ListOwnedQuery>,
) -> Json<ListOwnedResponse> {
    let wt_dir = Path::new(&state.repo_root).join(".worktrees");
    let mut worktrees = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&wt_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(data) = read_owner_file(&path) {
                let file_agent = data.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
                if file_agent == query.agent_id {
                    worktrees.push(OwnedWorktree {
                        path: path.to_string_lossy().to_string(),
                        task: data.get("task").and_then(|v| v.as_str()).map(String::from),
                        created_at: data
                            .get("created_at")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    });
                }
            }
        }
    }

    Json(ListOwnedResponse {
        agent_id: query.agent_id,
        worktrees,
    })
}

/// Handler: list all known workspaces in a JSON shape compatible with `cvg wave`.
pub async fn handle_list(
    State(state): State<Arc<WorkspaceState>>,
    Query(query): Query<ListWorkspacesQuery>,
) -> Json<ListWorkspacesResponse> {
    let wt_dir = Path::new(&state.repo_root).join(".worktrees");
    let mut workspaces = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&wt_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let owner_data = read_owner_file(&path);
            let owner = owner_data
                .as_ref()
                .and_then(|data| data.get("agent_id"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let task = owner_data
                .as_ref()
                .and_then(|data| data.get("task"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let wave_db_id = owner_data
                .as_ref()
                .and_then(|data| data.get("wave_db_id"))
                .and_then(|v| v.as_i64());

            workspaces.push(WorkspaceEntry {
                workspace_id: path
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("unknown-workspace")
                    .to_string(),
                path: path.to_string_lossy().to_string(),
                status: "active".to_string(),
                wave_db_id,
                owner,
                task,
            });
        }
    }

    workspaces.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));
    Json(ListWorkspacesResponse {
        plan_id: query.plan_id,
        workspaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn read_owner_file_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let owner = serde_json::json!({
            "agent_id": "test-agent",
            "created_at": "2026-04-07T10:00:00Z",
            "task": "Plan Zero W1"
        });
        fs::write(
            tmp.path().join(".worktree-owner"),
            serde_json::to_string_pretty(&owner).unwrap(),
        )
        .unwrap();
        let data = read_owner_file(tmp.path()).unwrap();
        assert_eq!(data["agent_id"], "test-agent");
    }

    #[test]
    fn read_owner_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_owner_file(tmp.path()).is_none());
    }

    #[test]
    fn resolve_worktree_path_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_str().unwrap();
        // Create the expected directory so canonicalize works
        std::fs::create_dir_all(tmp.path().join(".worktrees/task-42")).unwrap();
        let p = resolve_worktree_path(root, "task-42").unwrap();
        assert!(p.ends_with(".worktrees/task-42"));
    }

    #[test]
    fn resolve_worktree_path_rejects_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_str().unwrap();
        assert!(resolve_worktree_path(root, "../etc/passwd").is_err());
    }
}
