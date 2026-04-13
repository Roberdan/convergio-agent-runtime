//! Parse token usage from agent log output.
//!
//! Supports two formats:
//! - Claude Code: single JSON object with `usage` and `modelUsage` fields
//! - Copilot CLI: JSONL stream where last line has `type: "result"` with `usage`
//!   and individual `assistant.message` events have `outputTokens`

use serde_json::Value;

/// Extracted token usage from an agent session.
#[derive(Debug, Default)]
pub struct TokenUsage {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
    pub num_turns: i64,
    pub duration_ms: i64,
    /// "claude" or "copilot"
    pub backend: String,
}

/// Parse token usage from agent.log content.
/// Tries Claude format first, then Copilot JSONL format.
pub fn parse_agent_log(content: &str) -> Option<TokenUsage> {
    // Try Claude format: single JSON object
    if let Ok(v) = serde_json::from_str::<Value>(content) {
        if v.get("total_cost_usd").is_some() {
            return parse_claude_json(&v);
        }
    }

    // Try Copilot JSONL: parse line by line
    parse_copilot_jsonl(content)
}

fn parse_claude_json(v: &Value) -> Option<TokenUsage> {
    let usage = v.get("usage")?;
    let model_usage = v.get("modelUsage");

    let model = model_usage
        .and_then(|mu| mu.as_object())
        .and_then(|obj| obj.keys().next())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".into());

    Some(TokenUsage {
        model,
        input_tokens: usage
            .get("input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cache_read_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cache_creation_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cost_usd: v
            .get("total_cost_usd")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        num_turns: v.get("num_turns").and_then(|v| v.as_i64()).unwrap_or(0),
        duration_ms: v.get("duration_ms").and_then(|v| v.as_i64()).unwrap_or(0),
        backend: "claude".into(),
    })
}

fn parse_copilot_jsonl(content: &str) -> Option<TokenUsage> {
    let mut total_output = 0i64;
    let mut total_input = 0i64;
    let mut turns = 0i64;
    let mut duration_ms = 0i64;
    let mut premium_requests = 0i64;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "assistant.message" => {
                if let Some(data) = v.get("data") {
                    total_output += data
                        .get("outputTokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    total_input += data
                        .get("inputTokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                }
                turns += 1;
            }
            "result" => {
                if let Some(usage) = v.get("usage") {
                    premium_requests += usage
                        .get("premiumRequests")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    duration_ms += usage
                        .get("sessionDurationMs")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                }
            }
            _ => {}
        }
    }

    // If no output tokens found, nothing to report
    if total_output == 0 && premium_requests == 0 {
        return None;
    }

    // Estimate input tokens from premium_requests if not available per-message
    // Each premium request ~= 1 API call ~= avg 3K-10K input tokens
    if total_input == 0 && premium_requests > 0 {
        total_input = premium_requests.saturating_mul(5000); // conservative estimate
    }

    // Estimate cost: Opus pricing ~$15/M input, ~$75/M output
    let est_cost =
        (total_input as f64 * 15.0 / 1_000_000.0) + (total_output as f64 * 75.0 / 1_000_000.0);

    Some(TokenUsage {
        model: "copilot-opus".into(),
        input_tokens: total_input,
        output_tokens: total_output,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        cost_usd: est_cost,
        num_turns: turns,
        duration_ms,
        backend: "copilot".into(),
    })
}

/// Record token usage to the tracking DB table.
pub fn record_to_db(pool: &convergio_db::pool::ConnPool, agent_id: &str, usage: &TokenUsage) {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("token tracking: DB connection failed: {e}");
            return;
        }
    };
    let (plan_id, task_id): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT plan_id, task_id FROM art_agents WHERE id = ?1",
            rusqlite::params![agent_id],
            |row| Ok((row.get(0).ok(), row.get(1).ok())),
        )
        .unwrap_or((None, None));

    let total_input = usage.input_tokens + usage.cache_read_tokens + usage.cache_creation_tokens;
    let result = conn.execute(
        "INSERT INTO token_usage \
         (plan_id, wave_id, task_id, agent, model, input_tokens, output_tokens, \
          cost_usd, execution_host) \
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            plan_id,
            task_id,
            agent_id,
            usage.model,
            total_input,
            usage.output_tokens,
            usage.cost_usd,
            usage.backend,
        ],
    );
    if let Err(e) = result {
        tracing::warn!("token tracking: INSERT failed: {e}");
    }
}

#[cfg(test)]
#[path = "token_parser_tests.rs"]
mod tests;
