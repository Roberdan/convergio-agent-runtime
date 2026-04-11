//! WorkspaceExtension — impl Extension for workspace management.

use std::sync::Arc;

use convergio_db::pool::ConnPool;
use convergio_types::extension::{
    AppContext, ExtResult, Extension, Health, McpToolDef, ScheduledTask,
};
use convergio_types::manifest::{Capability, Manifest, ModuleKind};

use crate::codegraph_routes::codegraph_routes;
use crate::reaper::{find_repo_root, reap_cycle, STALE_THRESHOLD};
use crate::routes::{workspace_routes, WorkspaceState};

pub struct WorkspaceExtension {
    #[allow(dead_code)]
    pool: ConnPool,
}

impl WorkspaceExtension {
    pub fn new(pool: ConnPool) -> Self {
        Self { pool }
    }
}

impl Default for WorkspaceExtension {
    fn default() -> Self {
        let pool = convergio_db::pool::create_memory_pool().expect("in-memory pool for default");
        Self { pool }
    }
}

impl Extension for WorkspaceExtension {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "convergio-workspace".to_string(),
            description:
                "Workspace management: worktree reaper, auto-cleanup of orphaned git worktrees"
                    .to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            kind: ModuleKind::Extension,
            provides: vec![Capability {
                name: "worktree-reaper".to_string(),
                version: "1.0".to_string(),
                description: "Auto-cleanup of orphaned git worktrees older than 24h".to_string(),
            }],
            requires: vec![],
            agent_tools: vec![],
            required_roles: vec![],
        }
    }

    fn routes(&self, _ctx: &AppContext) -> Option<axum::Router> {
        let repo_root = std::env::current_dir()
            .ok()
            .and_then(|p| find_repo_root(&p))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".into());

        let state = Arc::new(WorkspaceState {
            repo_root: repo_root.clone(),
        });
        let cg_state = Arc::new(WorkspaceState { repo_root });
        Some(workspace_routes(state).merge(codegraph_routes(cg_state)))
    }

    fn on_start(&self, _ctx: &AppContext) -> ExtResult<()> {
        tracing::info!("workspace: worktree reaper registered (6h cron)");
        Ok(())
    }

    fn health(&self) -> convergio_types::extension::Health {
        Health::Ok
    }

    fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        vec![ScheduledTask {
            name: "worktree-reaper",
            cron: "0 */6 * * *",
        }]
    }

    fn on_scheduled_task(&self, task_name: &str) {
        if task_name != "worktree-reaper" {
            return;
        }
        let repo_root = std::env::current_dir()
            .ok()
            .and_then(|p| find_repo_root(&p))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        tokio::spawn(async move {
            let report = reap_cycle(&repo_root, STALE_THRESHOLD);
            tracing::info!(
                reaped = report.reaped.len(),
                errors = report.errors.len(),
                skipped = report.skipped,
                "workspace-reaper: scheduled run complete"
            );
        });
    }

    fn mcp_tools(&self) -> Vec<McpToolDef> {
        crate::mcp_defs::workspace_tools()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_correct_id() {
        let ext = WorkspaceExtension::default();
        let m = ext.manifest();
        assert_eq!(m.id, "convergio-workspace");
        assert!(!m.provides.is_empty());
    }

    #[test]
    fn scheduled_tasks_returns_reaper() {
        let ext = WorkspaceExtension::default();
        let tasks = ext.scheduled_tasks();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "worktree-reaper");
        assert_eq!(tasks[0].cron, "0 */6 * * *");
    }
}
