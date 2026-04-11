//! Resolve the git repository root.
//!
//! The daemon runs from `daemon/` (the cargo workspace root), but worktree
//! operations need the real git repo root (one level up).  Using `current_dir`
//! would give `~/GitHub/convergio/daemon`, and `.parent().join("worktrees")`
//! would place worktrees at `~/GitHub/convergio/worktrees/` — INSIDE the repo.
//!
//! Instead we call `git rev-parse --show-toplevel` which returns
//! `~/GitHub/convergio`, so `.parent().join("worktrees")` correctly resolves to
//! `~/GitHub/worktrees/` — OUTSIDE the repo.

use std::process::Command;

/// Return the git repo root as a `String`.
///
/// Tries `git rev-parse --show-toplevel` first; falls back to `CONVERGIO_REPO_ROOT`
/// env var, then to the current directory (last resort).
pub fn resolve_repo_root() -> String {
    // 1. Env override (useful for tests and non-git deployments)
    if let Ok(v) = std::env::var("CONVERGIO_REPO_ROOT") {
        if !v.is_empty() {
            return v;
        }
    }

    // 2. Ask git for the real top-level
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !root.is_empty() {
                return root;
            }
        }
    }

    // 3. Fallback: current directory (preserves old behaviour)
    tracing::warn!("resolve_repo_root: git rev-parse failed, falling back to current_dir");
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_returns_non_empty() {
        let root = resolve_repo_root();
        assert!(!root.is_empty());
    }

    #[test]
    fn env_override_takes_priority() {
        // SAFETY: test runs single-threaded (cargo test default), no other
        // thread reads this env var concurrently.
        let key = "CONVERGIO_REPO_ROOT";
        let prev = std::env::var(key).ok();
        unsafe { std::env::set_var(key, "/tmp/fake-repo") };
        let root = resolve_repo_root();
        // Restore
        match prev {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
        assert_eq!(root, "/tmp/fake-repo");
    }
}
