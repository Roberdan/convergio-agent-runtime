//! convergio-workspace — worktree reaper, ownership guard, code graph, workspace management.

pub mod codegraph;
pub mod codegraph_routes;
pub mod ext;
pub mod guard;
pub mod mcp_defs;
pub mod reaper;
pub mod routes;

pub use ext::WorkspaceExtension;
