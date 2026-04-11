//! Worktree ownership — write/read `.worktree-owner` files for agent isolation.
//!
//! When an agent creates a worktree, it stamps a `.worktree-owner` JSON file
//! so other agents can verify ownership before touching the worktree.

use std::path::Path;

use serde_json::json;

use crate::types::{RuntimeError, RuntimeResult};

/// Write `.worktree-owner` file into the worktree with agent identity.
/// This prevents other agents from accidentally modifying or destroying the worktree.
pub fn write_worktree_owner(wt_path: &Path, task_name: &str) -> RuntimeResult<()> {
    let agent_id =
        std::env::var("CONVERGIO_AGENT_NAME").unwrap_or_else(|_| "unknown-agent".to_string());
    let created_at = chrono::Utc::now().to_rfc3339();
    let owner = json!({
        "agent_id": agent_id,
        "created_at": created_at,
        "task": task_name,
    });
    let path = wt_path.join(".worktree-owner");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&owner).unwrap_or_default(),
    )
    .map_err(|e| RuntimeError::Internal(format!("write .worktree-owner: {e}")))?;
    Ok(())
}

/// Read `.worktree-owner` from a worktree path. Returns the parsed JSON value.
pub fn read_worktree_owner(wt_path: &Path) -> RuntimeResult<serde_json::Value> {
    let path = wt_path.join(".worktree-owner");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| RuntimeError::Internal(format!("read .worktree-owner: {e}")))?;
    serde_json::from_str(&content)
        .map_err(|e| RuntimeError::Internal(format!("parse .worktree-owner: {e}")))
}

/// Check if the given agent_id owns the worktree at wt_path.
pub fn check_worktree_owner(wt_path: &Path, agent_id: &str) -> RuntimeResult<bool> {
    let owner = read_worktree_owner(wt_path)?;
    Ok(owner.get("agent_id").and_then(|v| v.as_str()) == Some(agent_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn write_and_read_worktree_owner() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CONVERGIO_AGENT_NAME", "test-agent-42");
        write_worktree_owner(tmp.path(), "test-task").unwrap();
        let owner = read_worktree_owner(tmp.path()).unwrap();
        assert_eq!(owner["agent_id"], "test-agent-42");
        assert_eq!(owner["task"], "test-task");
        assert!(owner["created_at"].as_str().is_some());
    }

    #[test]
    fn check_worktree_owner_correct() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CONVERGIO_AGENT_NAME", "owner-agent");
        write_worktree_owner(tmp.path(), "my-task").unwrap();
        assert!(check_worktree_owner(tmp.path(), "owner-agent").unwrap());
        assert!(!check_worktree_owner(tmp.path(), "other-agent").unwrap());
    }

    #[test]
    fn read_worktree_owner_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_worktree_owner(tmp.path()).is_err());
    }

    #[test]
    fn owner_file_is_valid_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CONVERGIO_AGENT_NAME", "json-agent");
        write_worktree_owner(tmp.path(), "json-task").unwrap();
        let raw = fs::read_to_string(tmp.path().join(".worktree-owner")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(parsed.is_object());
        assert!(parsed.get("agent_id").is_some());
        assert!(parsed.get("created_at").is_some());
        assert!(parsed.get("task").is_some());
    }
}
