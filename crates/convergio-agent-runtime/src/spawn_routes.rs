//! HTTP endpoint for real agent spawning.
//!
//! POST /api/agents/spawn — creates worktree, writes instructions, launches process.
//! Rate limiting and alerting delegated to `spawn_guard` module.

use std::sync::Arc;

use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use axum::Router;
use convergio_db::pool::ConnPool;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::context_enrichment::enrich_with_file_context;
use crate::spawn_guard::{self, RateLimiter};
use crate::spawner;
use crate::types::SpawnRequest;

pub struct SpawnState {
    pub pool: ConnPool,
    pub repo_root: String,
    pub daemon_url: String,
    pub rate_limiter: RateLimiter,
    pub event_sink: Option<std::sync::Arc<dyn convergio_types::events::DomainEventSink>>,
}

pub fn spawn_routes(state: Arc<SpawnState>) -> Router {
    Router::new()
        .route("/api/agents/spawn", post(handle_spawn))
        .with_state(state)
}

#[derive(Deserialize)]
struct SpawnBody {
    agent_name: String,
    org_id: String,
    task_id: Option<i64>,
    #[serde(default)]
    instructions: String,
    /// Model override (e.g. "claude-opus-4-6"). If None, uses tier default.
    model: Option<String>,
    /// Model tier: t1 (opus), t2 (sonnet), t3 (haiku), t4 (local).
    #[serde(default = "default_tier")]
    tier: String,
    #[serde(default = "default_budget")]
    budget_usd: f64,
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
    #[serde(default)]
    priority: i32,
    /// Override repo root for spawning in external repos.
    repo_override: Option<String>,
    /// Override the instruction file name (default: TASK.md).
    instruction_file: Option<String>,
    /// Dry-run mode: validate allocation + worktree creation without spawning
    /// a real process. Used by doctor E2E checks to avoid burning tokens/CPU.
    #[serde(default)]
    dry_run: bool,
}

fn default_tier() -> String {
    "t2".into()
}
fn default_budget() -> f64 {
    10.0
}
fn default_timeout() -> u64 {
    7200
}

async fn handle_spawn(
    State(state): State<Arc<SpawnState>>,
    Json(body): Json<SpawnBody>,
) -> (axum::http::StatusCode, Json<Value>) {
    // Input validation: agent_name (alphanumeric + dash/underscore, max 64 chars)
    if body.agent_name.is_empty()
        || body.agent_name.len() > 64
        || !body
            .agent_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error": "agent_name must be 1-64 alphanumeric/dash/underscore chars"})),
        );
    }

    // Input validation: org_id
    if body.org_id.is_empty()
        || body.org_id.len() > 64
        || !body
            .org_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error": "org_id must be 1-64 alphanumeric/dash/underscore chars"})),
        );
    }

    // Input validation: repo_override (no traversal, must be absolute)
    if let Some(ref repo) = body.repo_override {
        if repo.contains("..") || !repo.starts_with('/') {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"error": "repo_override must be an absolute path without traversal"})),
            );
        }
    }

    // Input validation: instruction_file (simple filename, no path separators)
    if let Some(ref f) = body.instruction_file {
        if f.contains('/') || f.contains('\\') || f.contains("..") || f.is_empty() {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"error": "instruction_file must be a simple filename"})),
            );
        }
    }

    // Rate limit check (incident-prevention)
    match spawn_guard::check_rate_limit(&state.rate_limiter, &body.org_id).await {
        Err(msg) => {
            tracing::warn!(org = body.org_id.as_str(), "spawn rate limit exceeded");
            return (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": msg})),
            );
        }
        Ok(recent_count) => {
            spawn_guard::maybe_fire_alert(&body.org_id, recent_count);
        }
    }

    // Model tier enforcement: t1 (Opus), t2 (Sonnet), t3 (Copilot) allowed.
    let allowed_tiers = ["t1", "t2", "t3"];
    if !allowed_tiers.contains(&body.tier.as_str()) && !is_non_code_output_type(&body.instructions)
    {
        tracing::warn!(
            agent = body.agent_name.as_str(),
            tier = body.tier.as_str(),
            "rejected tier — use t1 (Opus), t2 (Sonnet), or t3 (Copilot)"
        );
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error": "Use tier=t1 (Opus), tier=t2 (Sonnet), or tier=t3 (Copilot)."})),
        );
    }

    let result = do_spawn(&state, body).await;
    (axum::http::StatusCode::OK, result)
}

async fn do_spawn(state: &Arc<SpawnState>, body: SpawnBody) -> Json<Value> {
    let repo_path = body.repo_override.as_deref().unwrap_or(&state.repo_root);
    let repo_root = std::path::Path::new(repo_path);
    let wt_name = format!("agent-{}", &body.agent_name);

    // 1. Register in DB
    let agent_id = {
        let conn = match state.pool.get() {
            Ok(c) => c,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let hostname = gethostname();
        let req = SpawnRequest {
            agent_name: body.agent_name.clone(),
            org_id: body.org_id.clone(),
            task_id: body.task_id,
            capabilities: vec![],
            model_preference: body.model.clone(),
            budget_usd: body.budget_usd,
            priority: body.priority,
        };
        match crate::allocator::spawn(&conn, &req, &hostname) {
            Ok(id) => id,
            Err(e) => return Json(json!({"error": format!("allocator: {e}")})),
        }
    };

    // 2. Create worktree
    let workspace = match spawner::create_worktree(repo_root, &wt_name) {
        Ok(p) => p,
        Err(e) => return Json(json!({"error": format!("worktree: {e}"), "agent_id": agent_id})),
    };

    // 3. Dry-run: validate infra (DB + worktree) then clean up — no process spawned.
    //    Used by doctor E2E checks to avoid burning tokens/CPU.
    if body.dry_run {
        tracing::info!(agent_id = agent_id.as_str(), "dry-run spawn — cleaning up");
        let _ = spawner::cleanup_worktree(repo_root, &workspace);
        return Json(json!({
            "ok": true,
            "agent_id": agent_id,
            "dry_run": true,
            "backend": "none (dry-run)",
        }));
    }

    // 4. Write instructions (skip if using a pre-existing instruction file)
    if body.instruction_file.is_none() {
        // Enrich with workspace context (who works on what files)
        let enriched = super::spawn_enrich::enrich_with_workspace_context(
            &state.daemon_url,
            &body.instructions,
        )
        .await;
        // Enrich with relevant knowledge context
        let enriched =
            super::spawn_enrich::enrich_with_knowledge(&state.daemon_url, &enriched, &body.org_id)
                .await;
        // Enrich with actual file contents mentioned in instructions
        let enriched = enrich_with_file_context(&workspace, &enriched);
        if let Err(e) = spawner::write_instructions(&workspace, &enriched) {
            return Json(json!({"error": format!("instructions: {e}"), "agent_id": agent_id}));
        }
    }

    // 5. Choose backend via tier
    let backend = spawner::backend_for_tier(&body.tier, body.model.as_deref());

    // 6. Spawn process
    let env_vars = [
        ("CONVERGIO_AGENT_NAME", body.agent_name.as_str()),
        ("CONVERGIO_ORG", body.org_id.as_str()),
        (
            "CONVERGIO_TASK_ID",
            &body.task_id.map(|id| id.to_string()).unwrap_or_default(),
        ),
        ("CONVERGIO_DAEMON_URL", state.daemon_url.as_str()),
        ("CONVERGIO_AGENT_ID", agent_id.as_str()),
    ];
    let result = spawner::spawn_process(
        &workspace,
        &backend,
        &env_vars,
        body.timeout_secs,
        body.instruction_file.as_deref(),
    );

    match result {
        Ok(spawned) => {
            // 6. Activate agent in DB
            if let Ok(conn) = state.pool.get() {
                let _ = crate::allocator::activate(&conn, &agent_id);
                // Store PID for reaper
                let _ = conn.execute(
                    "UPDATE art_agents SET workspace_path = ?1 WHERE id = ?2",
                    rusqlite::params![workspace.to_string_lossy().as_ref(), agent_id],
                );
            }
            // 7. Broadcast FilesClaimed intent (OODA: other agents see what we claim)
            super::spawn_enrich::emit_files_claimed(
                &state.pool,
                state.event_sink.as_ref(),
                body.task_id,
                &body.agent_name,
                &body.org_id,
            );
            // 8. Register in IPC so all agents are visible in `cvg who`
            let hostname = gethostname();
            let backend_type = spawned.backend.split(':').next().unwrap_or("unknown");
            if let Err(e) = convergio_ipc::agents::register(
                &state.pool,
                &body.agent_name,
                backend_type,
                Some(spawned.pid),
                &hostname,
                Some(&format!("org={} task={:?}", body.org_id, body.task_id)),
                None,
            ) {
                tracing::warn!(agent = body.agent_name.as_str(), "IPC register failed: {e}");
            }
            // 8. Start monitor — watches process, handles push/PR on completion
            crate::spawn_monitor::monitor_agent(
                state.pool.clone(),
                agent_id.clone(),
                spawned.pid,
                workspace.to_string_lossy().to_string(),
                state.repo_root.clone(),
                body.agent_name.clone(),
            );
            tracing::info!(
                agent_id = agent_id.as_str(),
                pid = spawned.pid,
                backend = spawned.backend.as_str(),
                workspace = %workspace.display(),
                "agent process spawned + monitor attached"
            );
            Json(json!({
                "ok": true,
                "agent_id": agent_id,
                "pid": spawned.pid,
                "backend": spawned.backend,
                "workspace": workspace.to_string_lossy(),
            }))
        }
        Err(e) => {
            tracing::error!(agent_id = agent_id.as_str(), "spawn failed: {e}");
            Json(json!({"error": format!("spawn: {e}"), "agent_id": agent_id}))
        }
    }
}

fn gethostname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Check if the task is a non-code output type (document, analysis, review).
fn is_non_code_output_type(instructions: &str) -> bool {
    let lower = instructions.to_lowercase();
    lower.contains("output_type: document")
        || lower.contains("output_type: analysis")
        || lower.contains("output_type: review")
}

/// Resolve the model ID based on task type, effort level, and role.
pub fn resolve_model(task_type: &str, effort_level: u8, role: &str) -> &'static str {
    let model = if role == "thor" || role == "validator" {
        "claude-sonnet-4.6"
    } else if matches!(task_type, "planning" | "review" | "analysis") || effort_level >= 3 {
        "claude-opus-4.6"
    } else {
        "claude-sonnet-4.6"
    };
    tracing::info!(task_type, effort_level, role, model, "model routing");
    model
}
