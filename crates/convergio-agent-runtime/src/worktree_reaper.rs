//! Worktree reaper — removes orphaned `.worktrees/` entries.
//!
//! - `reap_orphaned_worktrees(repo_root)` — finds entries older than 24h
//!   with no running agent, removes them.
//! - Route: `POST /api/workspace/reap` (manual trigger)
//! - Scheduled task: every 6h via cron

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use axum::Router;
use serde::Serialize;
use serde_json::json;

use convergio_db::pool::ConnPool;

/// State for the worktree reaper route.
pub struct WorktreeReaperState {
    pub pool: ConnPool,
    pub repo_root: String,
}

/// Report from a single reap cycle.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WorktreeReapReport {
    pub scanned: usize,
    pub removed: usize,
    pub skipped_active: usize,
    pub errors: Vec<String>,
}

/// Build the worktree reaper router.
pub fn worktree_reaper_routes(state: Arc<WorktreeReaperState>) -> Router {
    Router::new()
        .route("/api/workspace/reap", post(handle_reap))
        .with_state(state)
}

async fn handle_reap(State(state): State<Arc<WorktreeReaperState>>) -> Json<serde_json::Value> {
    let report = reap_orphaned_worktrees(&state.repo_root, &state.pool);
    Json(json!({
        "ok": true,
        "report": report,
    }))
}

/// Scan `.worktrees/` for directories older than 24 hours with no running
/// agent and remove them.
pub fn reap_orphaned_worktrees(repo_root: &str, pool: &ConnPool) -> WorktreeReapReport {
    let mut report = WorktreeReapReport::default();
    // Worktrees live outside the repo (see spawner.rs for rationale)
    let wt_dir = Path::new(repo_root)
        .parent()
        .unwrap_or(Path::new(repo_root))
        .join("worktrees");

    let entries = match std::fs::read_dir(&wt_dir) {
        Ok(e) => e,
        Err(_) => return report,
    };

    // Incident-prevention: reduced from 24h to 2h
    let cutoff = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    let active_paths = active_worktree_paths(pool);

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        report.scanned += 1;

        // Check modification time
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if modified > cutoff {
            continue; // Not old enough
        }

        // Check if any running agent references this worktree
        let path_str = path.to_string_lossy().to_string();
        if active_paths.contains(&path_str) {
            report.skipped_active += 1;
            continue;
        }

        // Remove the orphaned worktree (git-aware cleanup)
        tracing::info!(path = %path_str, "removing orphaned worktree");
        let wt_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let repo = Path::new(repo_root);

        // Try git worktree remove first (cleans git metadata properly)
        let git_rm = std::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&path)
            .current_dir(repo)
            .output();
        let removed_by_git = git_rm.map(|o| o.status.success()).unwrap_or(false);

        // Fallback to rm -rf if git remove failed
        if !removed_by_git {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!(path = %path_str, error = %e, "failed to remove worktree");
                report.errors.push(format!("{path_str}: {e}"));
                continue;
            }
        }

        // Prune dangling worktree references
        let _ = std::process::Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo)
            .output();

        // Delete the orphaned agent branch
        let branch = format!("agent/{wt_name}");
        let branch_exists = std::process::Command::new("git")
            .args(["rev-parse", "--verify"])
            .arg(&branch)
            .current_dir(repo)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if branch_exists {
            let _ = std::process::Command::new("git")
                .args(["branch", "-D"])
                .arg(&branch)
                .current_dir(repo)
                .output();
            tracing::info!(branch = %branch, "deleted orphaned agent branch");
        }

        report.removed += 1;
    }

    if report.removed > 0 {
        tracing::info!(
            scanned = report.scanned,
            removed = report.removed,
            skipped = report.skipped_active,
            "worktree reaper cycle complete"
        );
    }

    report
}

/// Query active agent workspace paths from the runtime DB.
fn active_worktree_paths(pool: &ConnPool) -> Vec<String> {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut stmt = match conn.prepare(
        "SELECT workspace_path FROM art_agents \
         WHERE stage IN ('running', 'spawning', 'borrowed') \
         AND workspace_path IS NOT NULL",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Spawn a background loop that reaps every 30 minutes.
pub fn spawn_worktree_reaper(pool: ConnPool, repo_root: String) -> tokio::task::JoinHandle<()> {
    let interval = Duration::from_secs(30 * 60);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await; // skip immediate tick
        loop {
            ticker.tick().await;
            let report = reap_orphaned_worktrees(&repo_root, &pool);
            if report.removed > 0 || !report.errors.is_empty() {
                tracing::info!(
                    removed = report.removed,
                    errors = report.errors.len(),
                    "worktree reaper background cycle"
                );
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_pool() -> ConnPool {
        let pool = convergio_db::pool::create_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        for m in crate::schema::migrations() {
            conn.execute_batch(m.up).unwrap();
        }
        pool
    }

    #[test]
    fn reap_empty_dir_returns_zero() {
        let pool = setup_pool();
        let tmp = tempfile::tempdir().unwrap();
        // Worktrees live in parent/worktrees/, so simulate with repo as subdir
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(tmp.path().join("worktrees")).unwrap();
        let report = reap_orphaned_worktrees(&repo.to_string_lossy(), &pool);
        assert_eq!(report.scanned, 0);
        assert_eq!(report.removed, 0);
    }

    #[test]
    fn reap_skips_recent_worktrees() {
        let pool = setup_pool();
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let wt_dir = tmp.path().join("worktrees");
        std::fs::create_dir_all(wt_dir.join("task-42")).unwrap();
        let report = reap_orphaned_worktrees(&repo.to_string_lossy(), &pool);
        assert_eq!(report.scanned, 1);
        assert_eq!(report.removed, 0); // too recent
    }

    #[test]
    fn active_worktree_paths_empty_on_clean_db() {
        let pool = setup_pool();
        let paths = active_worktree_paths(&pool);
        assert!(paths.is_empty());
    }

    #[test]
    fn reap_report_serializes() {
        let report = WorktreeReapReport {
            scanned: 5,
            removed: 2,
            skipped_active: 1,
            errors: vec!["test".into()],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"scanned\":5"));
    }

    #[test]
    fn worktree_reaper_routes_build() {
        let pool = setup_pool();
        let state = Arc::new(WorktreeReaperState {
            pool,
            repo_root: "/tmp".into(),
        });
        let _router = worktree_reaper_routes(state);
    }
}
