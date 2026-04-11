//! MCP tool definitions for the workspace extension.

use convergio_types::extension::McpToolDef;
use serde_json::json;

pub fn workspace_tools() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            name: "cvg_check_worktree_owner".into(),
            description: "Check who owns a worktree. Must verify before modifying.".into(),
            method: "POST".into(),
            path: "/api/workspace/check-owner".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "worktree_path": {"type": "string", "description": "Path to the worktree"}
                },
                "required": ["worktree_path"]
            }),
            min_ring: "community".into(),
            path_params: vec![],
        },
        McpToolDef {
            name: "cvg_workspace_gc".into(),
            description: "Run workspace garbage collection: remove stale worktrees, prune dead branches and remote refs.".into(),
            method: "POST".into(),
            path: "/api/workspace/gc".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "threshold_minutes": {
                        "type": "integer",
                        "description": "Min age in minutes before a worktree is reaped (default: 60)"
                    }
                }
            }),
            min_ring: "trusted".into(),
            path_params: vec![],
        },
        McpToolDef {
            name: "cvg_codegraph_expand".into(),
            description: "Expand file list through dependency graph — find all affected packages."
                .into(),
            method: "POST".into(),
            path: "/api/codegraph/expand".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "File paths to expand (e.g. daemon/crates/convergio-mcp/src/profile.rs)"
                    }
                },
                "required": ["files"]
            }),
            min_ring: "community".into(),
            path_params: vec![],
        },
        McpToolDef {
            name: "cvg_codegraph_package_deps".into(),
            description: "Get package-level dependency map for the workspace.".into(),
            method: "GET".into(),
            path: "/api/codegraph/package-deps".into(),
            input_schema: json!({"type": "object", "properties": {}}),
            min_ring: "community".into(),
            path_params: vec![],
        },
    ]
}
