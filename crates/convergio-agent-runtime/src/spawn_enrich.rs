//! Knowledge enrichment for agent spawn instructions.

/// Query the knowledge vector store and prepend relevant context to instructions.
/// Falls back silently to original instructions if the API is unavailable.
pub async fn enrich_with_knowledge(daemon_url: &str, instructions: &str, org_id: &str) -> String {
    let url = format!("{daemon_url}/api/knowledge/search");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let token = std::env::var("CONVERGIO_AUTH_TOKEN").ok();
    let query: String = instructions.chars().take(200).collect();

    let mut req = client.post(&url).json(&serde_json::json!({
        "query": query,
        "limit": 5,
        "org_id": org_id,
    }));
    if let Some(t) = &token {
        req = req.bearer_auth(t);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "knowledge enrichment unavailable");
            return instructions.to_string();
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return instructions.to_string(),
    };

    let results = match body.get("results").and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return instructions.to_string(),
    };

    let mut context = String::from("## Relevant Knowledge Context\n\n");
    for (i, r) in results.iter().enumerate() {
        let content = r
            .pointer("/entry/content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let score = r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if score > 0.3 && !content.is_empty() {
            context.push_str(&format!("{}. {}\n\n", i + 1, content));
        }
    }

    if context.len() > 40 {
        tracing::info!(
            hits = results.len(),
            "injected knowledge context into agent instructions"
        );
        format!("{context}---\n\n{instructions}")
    } else {
        instructions.to_string()
    }
}

/// Query in-progress tasks and prepend workspace context showing who works on what.
/// This prevents file conflicts by making agents aware of each other's claimed files.
pub async fn enrich_with_workspace_context(daemon_url: &str, instructions: &str) -> String {
    let url = format!("{daemon_url}/api/plan-db/tasks/in-progress");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return instructions.to_string(),
    };
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return instructions.to_string(),
    };
    let tasks = match body.get("tasks").and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return instructions.to_string(),
    };

    let mut ctx = String::from("## Workspace Context (DO NOT touch these files)\n\n");
    let mut any = false;
    for t in tasks {
        let agent = t
            .get("executor_agent")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("?");
        let files = t
            .get("claimed_files")
            .and_then(|v| v.as_str())
            .unwrap_or("[]");
        if files != "[]" {
            ctx.push_str(&format!("- {agent} ({title}) → {files}\n"));
            any = true;
        }
    }
    if any {
        tracing::info!(active_tasks = tasks.len(), "injected workspace context");
        format!("{ctx}\n---\n\n{instructions}")
    } else {
        instructions.to_string()
    }
}

/// Emit FilesClaimed event after agent activation (OODA intent broadcast).
pub fn emit_files_claimed(
    pool: &convergio_db::pool::ConnPool,
    sink: Option<&std::sync::Arc<dyn convergio_types::events::DomainEventSink>>,
    task_id: Option<i64>,
    agent_name: &str,
    org_id: &str,
) {
    let (Some(sink), Some(tid)) = (sink, task_id) else {
        return;
    };
    let Ok(conn) = pool.get() else { return };
    let files: String = conn
        .query_row(
            "SELECT claimed_files FROM tasks WHERE id = ?1",
            [tid],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "[]".into());
    let paths: Vec<String> = serde_json::from_str(&files).unwrap_or_default();
    if paths.is_empty() {
        return;
    }
    use convergio_types::events::*;
    sink.emit(make_event(
        agent_name,
        EventKind::FilesClaimed {
            task_id: tid,
            agent: agent_name.to_string(),
            file_paths: paths,
        },
        EventContext {
            org_id: Some(org_id.to_string()),
            plan_id: None,
            task_id: Some(tid),
        },
    ));
}
