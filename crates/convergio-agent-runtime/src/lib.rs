//! convergio-agent-runtime — Agent runtime: memory model + concurrency.
//!
//! The daemon manages agents like a compiler manages memory:
//! allocation (spawn), ownership (org), borrowing (delegation),
//! lifetime (heartbeat), GC (reaper).
//!
//! Implements Extension: provides scheduling, isolation, and lifecycle
//! management for AI agents.

pub mod adaptation;
pub mod adaptation_routes;
pub mod allocator;
pub mod concurrency;
pub mod context;
pub mod context_enrichment;
pub mod context_routes;
pub mod delegation;
pub mod ext;
pub mod harness;
pub mod heartbeat;
pub mod mcp_defs;
pub mod monitor_helpers;
pub mod reaper;
pub mod repo_root;
pub mod respawn;
pub mod routes;
pub mod scheduler;
pub mod schema;
pub mod scope;
pub mod spawn_backend;
pub mod spawn_enrich;
pub mod spawn_guard;
pub mod spawn_monitor;
pub mod spawn_routes;
pub mod spawner;
pub mod token_parser;
pub mod types;
pub mod worktree_owner;
pub mod worktree_reaper;

pub use ext::AgentRuntimeExtension;
