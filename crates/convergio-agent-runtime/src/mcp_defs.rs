//! MCP tool definitions for the agent runtime extension.

use convergio_types::extension::McpToolDef;
use serde_json::json;

pub fn agent_runtime_tools() -> Vec<McpToolDef> {
    vec![McpToolDef {
        name: "cvg_spawn_agent".into(),
        description: "Spawn a sub-agent to work on a task. \
                      Uses Opus model. Returns agent name and status."
            .into(),
        method: "POST".into(),
        path: "/api/agents/spawn".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "agent_name": {"type": "string", "description": "Unique agent name"},
                "org_id": {"type": "string", "description": "Organization ID"},
                "task_id": {"type": "integer", "description": "Associated plan task ID"},
                "instructions": {"type": "string", "description": "What the agent should do"},
                "tier": {"type": "string", "description": "Agent tier (default: t1/Opus)", "default": "t1"},
                "budget_usd": {"type": "number", "description": "Max spend in USD", "default": 10},
                "timeout_secs": {"type": "integer", "description": "Max runtime seconds", "default": 3600},
                "dry_run": {"type": "boolean", "description": "If true, validate only — don't actually spawn"}
            },
            "required": ["agent_name", "org_id", "instructions"]
        }),
        min_ring: "trusted".into(),
        path_params: vec![],
    }]
}
