//! HTTP API routes for convergio-agent-runtime.

use axum::Router;

/// Returns the router for this crate's API endpoints.
pub fn routes() -> Router {
    Router::new()
    // .route("/api/agent-runtime/health", get(health))
}
