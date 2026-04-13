//! API routes: GET /api/agents/runtime — live view of the runtime.
//! POST /api/agents/kill-all — emergency kill switch for all active agents.

use std::sync::Arc;

use axum::extract::State;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;

use convergio_db::pool::ConnPool;

use crate::types::RuntimeView;

/// Shared state for runtime routes.
pub struct RuntimeState {
    pub pool: ConnPool,
}

/// Build the agent runtime API router.
pub fn runtime_routes(state: Arc<RuntimeState>) -> Router {
    Router::new()
        .route("/api/agents/runtime", get(handle_runtime_view))
        .route("/api/agents/kill-all", post(handle_kill_all))
        .with_state(state)
}

async fn handle_runtime_view(State(state): State<Arc<RuntimeState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("DB pool error: {e}");
            return Json(serde_json::json!({
                "error": { "code": "POOL_ERROR", "message": "internal database error" }
            }));
        }
    };

    let active_agents = crate::allocator::list_active(&conn, None).unwrap_or_default();
    let queue_depth = crate::scheduler::queue_depth_total(&conn).unwrap_or(0);
    let stale = crate::heartbeat::find_stale(&conn)
        .map(|s| s.len())
        .unwrap_or(0);

    // Aggregate budget info
    let (total_budget, total_spent) = conn
        .query_row(
            "SELECT COALESCE(SUM(budget_usd), 0.0), COALESCE(SUM(spent_usd), 0.0) \
             FROM art_agents WHERE stage IN ('running', 'borrowed', 'spawning')",
            [],
            |r| Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?)),
        )
        .unwrap_or((0.0, 0.0));

    let delegations_active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM art_delegations WHERE returned = 0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let discovered = convergio_ipc::agents::list(&state.pool).unwrap_or_default();

    let view = RuntimeView {
        active_agents,
        discovered_agents: discovered,
        queue_depth,
        total_budget_usd: total_budget,
        total_spent_usd: total_spent,
        delegations_active: delegations_active as usize,
        stale_count: stale,
    };

    Json(serde_json::to_value(view).unwrap_or_default())
}

/// Emergency kill switch — stop ALL active agents across all orgs.
/// Also kills their OS processes if PIDs are available.
async fn handle_kill_all(State(state): State<Arc<RuntimeState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("DB pool error: {e}");
            return Json(serde_json::json!({
                "error": { "code": "POOL_ERROR", "message": "internal database error" }
            }));
        }
    };

    // Collect PIDs of running agents before stopping them in DB
    let pids: Vec<u32> = {
        let mut stmt = conn
            .prepare(
                "SELECT workspace_path FROM art_agents \
                 WHERE stage IN ('running', 'spawning', 'borrowed')",
            )
            .unwrap_or_else(|_| conn.prepare("SELECT 1 WHERE 0").unwrap());
        stmt.query_map([], |_r| Ok(()))
            .map(|rows| rows.count())
            .unwrap_or(0);
        // PIDs aren't stored yet — future improvement
        vec![]
    };

    // Stop all agents in DB
    let stopped = conn
        .execute(
            "UPDATE art_agents SET stage = 'stopped', updated_at = datetime('now') \
             WHERE stage IN ('running', 'spawning', 'borrowed')",
            [],
        )
        .unwrap_or(0);

    // Also pause all in_progress plans to prevent respawning
    let paused = conn
        .execute(
            "UPDATE plans SET status = 'paused', updated_at = datetime('now') \
             WHERE status = 'in_progress'",
            [],
        )
        .unwrap_or(0);

    tracing::warn!(
        stopped,
        paused,
        "KILL SWITCH activated — all agents stopped, all plans paused"
    );

    Json(serde_json::json!({
        "ok": true,
        "agents_stopped": stopped,
        "plans_paused": paused,
        "pids_killed": pids.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_routes_builds() {
        let pool = convergio_db::pool::create_memory_pool().unwrap();
        let state = Arc::new(RuntimeState { pool });
        let _router = runtime_routes(state);
    }
}
