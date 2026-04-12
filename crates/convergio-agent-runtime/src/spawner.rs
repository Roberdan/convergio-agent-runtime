//! Real agent process spawner — creates worktree, writes instructions, launches process.
//!
//! This is the bridge between "daemon decides" and "daemon acts".
//! Supports multiple backends: Claude CLI, API call, script.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::types::{RuntimeError, RuntimeResult};
use crate::worktree_owner::write_worktree_owner;

/// Maximum concurrent worktrees allowed (incident-prevention).
const MAX_WORKTREES: usize = 5;

/// Which backend to use for the agent process.
#[derive(Debug, Clone)]
pub enum SpawnBackend {
    /// Claude Code CLI with tool access (for complex tasks needing file ops).
    ClaudeCli { model: String },
    /// GitHub Copilot CLI for mechanical/delegation tasks (cheaper than Claude).
    CopilotCli { model: Option<String> },
    /// Direct shell command (for deterministic/scripted tasks).
    Script { command: String, args: Vec<String> },
}

/// Result of spawning an agent process.
#[derive(Debug)]
pub struct SpawnedProcess {
    pub pid: u32,
    pub workspace: PathBuf,
    pub backend: String,
}

/// Create a git worktree for the agent. Returns the worktree path.
///
/// If a stale worktree or branch exists from a previous failed attempt,
/// cleans it up automatically before creating the new one.
pub fn create_worktree(repo_root: &Path, name: &str) -> RuntimeResult<PathBuf> {
    // Validate name has no path traversal
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(RuntimeError::Internal(
            "worktree name contains unsafe characters".into(),
        ));
    }
    // Worktrees go OUTSIDE the repo to avoid Claude Code loading .claude/ rules
    // from every worktree (wastes context, causes confusion with 10+ worktrees).
    let wt_base = repo_root.parent().unwrap_or(repo_root).join("worktrees");
    let _ = std::fs::create_dir_all(&wt_base);

    // Enforce worktree quota (incident-prevention fix)
    if let Ok(entries) = std::fs::read_dir(&wt_base) {
        let count = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .count();
        if count >= MAX_WORKTREES {
            return Err(RuntimeError::Internal(format!(
                "worktree quota exceeded: {count}/{MAX_WORKTREES} active"
            )));
        }
    }

    let wt_path = wt_base.join(name);
    let branch = format!("agent/{name}");

    if wt_path.exists() {
        tracing::info!("removing stale worktree {}", wt_path.display());
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt_path)
            .current_dir(repo_root)
            .output();
    }
    let branch_check = Command::new("git")
        .args(["rev-parse", "--verify"])
        .arg(&branch)
        .current_dir(repo_root)
        .output();
    if branch_check.map(|o| o.status.success()).unwrap_or(false) {
        tracing::info!("deleting stale branch {branch}");
        let _ = Command::new("git")
            .args(["branch", "-D"])
            .arg(&branch)
            .current_dir(repo_root)
            .output();
    }

    let output = Command::new("git")
        .args(["worktree", "add", "-b", &branch])
        .arg(&wt_path)
        .arg("HEAD")
        .current_dir(repo_root)
        .output()
        .map_err(|e| RuntimeError::Internal(format!("git worktree: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RuntimeError::Internal(format!(
            "git worktree failed: {stderr}"
        )));
    }

    // Post-creation validation: verify the worktree is a valid git checkout
    validate_worktree(&wt_path, &branch)?;

    // Layer 1: write ownership file so other agents know who owns this worktree
    write_worktree_owner(&wt_path, name)?;
    // Layer 2: override MCP profile to CEO for delegated agents
    // The repo .mcp.json uses "compact" — delegated agents need the single
    // cvg_ceo tool that routes everything via LLM (saves ~10K tokens/session).
    write_mcp_ceo_profile(&wt_path)?;

    Ok(wt_path)
}

/// Write task instructions and baseline script to the worktree.
pub fn write_instructions(workspace: &Path, instructions: &str) -> RuntimeResult<PathBuf> {
    use crate::harness;
    convergio_types::platform_paths::validate_path_components(workspace)
        .map_err(RuntimeError::Internal)?;
    let path = workspace.join("TASK.md");
    let content = format!(
        "{header}\n\n---\n\n{instructions}",
        header = harness::DELEGATION_HEADER
    );
    std::fs::write(&path, &content)
        .map_err(|e| RuntimeError::Internal(format!("write TASK.md: {e}")))?;

    // Write baseline test script (Fase 49: harness engineering)
    harness::write_baseline_script(workspace)?;

    Ok(path)
}

/// Spawn the agent process in the workspace.
/// `instruction_file` overrides the default "TASK.md" prompt target.
pub fn spawn_process(
    workspace: &Path,
    backend: &SpawnBackend,
    env_vars: &[(&str, &str)],
    _timeout_secs: u64,
    instruction_file: Option<&str>,
) -> RuntimeResult<SpawnedProcess> {
    let child = match backend {
        SpawnBackend::ClaudeCli { model } => {
            // Learning #7: short prompt, instructions in file
            // Learning #19+20: launchd has minimal PATH — use absolute paths
            // Don't use external `timeout` — it's also missing. Reaper handles timeouts.
            let claude_bin = resolve_claude_path();
            let mut cmd = Command::new(&claude_bin);
            cmd.args(["--dangerously-skip-permissions"]);
            cmd.args(["--model", model]);
            // Learning: claude -p sometimes hangs after completing its task.
            // --max-turns caps conversation turns so the process exits reliably.
            cmd.args(["--max-turns", "50"]);
            cmd.args(["--output-format", "json"]);
            let target = instruction_file.unwrap_or("TASK.md");
            let prompt = format!("Leggi {target} per le istruzioni. Poi inizia.");
            cmd.args(["-p", &prompt]);
            cmd.current_dir(workspace);
            for (k, v) in env_vars {
                cmd.env(k, v);
            }
            // Log output to files in worktree (Learning #18: /dev/null hides errors)
            let log_out = std::fs::File::create(workspace.join("agent.log"))
                .map_err(|e| RuntimeError::Internal(format!("create log: {e}")))?;
            let log_err = std::fs::File::create(workspace.join("agent.err"))
                .map_err(|e| RuntimeError::Internal(format!("create err: {e}")))?;
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::from(log_out));
            cmd.stderr(std::process::Stdio::from(log_err));
            cmd.spawn()
                .map_err(|e| RuntimeError::Internal(format!("spawn claude: {e}")))?
        }
        SpawnBackend::CopilotCli { model } => {
            let gh_bin = resolve_gh_path();
            let mut cmd = Command::new(&gh_bin);
            cmd.args(["copilot", "--", "--yolo"]);
            if let Some(m) = &model {
                cmd.args(["--model", m]);
            }
            let target = instruction_file.unwrap_or("TASK.md");
            let prompt = format!("Leggi {target} per le istruzioni. Poi inizia.");
            cmd.args(["-p", &prompt]);
            cmd.current_dir(workspace);
            for (k, v) in env_vars {
                cmd.env(k, v);
            }
            let log_out = std::fs::File::create(workspace.join("agent.log"))
                .map_err(|e| RuntimeError::Internal(format!("create log: {e}")))?;
            let log_err = std::fs::File::create(workspace.join("agent.err"))
                .map_err(|e| RuntimeError::Internal(format!("create err: {e}")))?;
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::from(log_out));
            cmd.stderr(std::process::Stdio::from(log_err));
            cmd.spawn()
                .map_err(|e| RuntimeError::Internal(format!("spawn copilot: {e}")))?
        }
        SpawnBackend::Script { command, args } => {
            let mut cmd = Command::new(command);
            cmd.args(args);
            cmd.current_dir(workspace);
            for (k, v) in env_vars {
                cmd.env(k, v);
            }
            let log_out = std::fs::File::create(workspace.join("agent.log"))
                .map_err(|e| RuntimeError::Internal(format!("create log: {e}")))?;
            let log_err = std::fs::File::create(workspace.join("agent.err"))
                .map_err(|e| RuntimeError::Internal(format!("create err: {e}")))?;
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::from(log_out));
            cmd.stderr(std::process::Stdio::from(log_err));
            cmd.spawn()
                .map_err(|e| RuntimeError::Internal(format!("spawn script: {e}")))?
        }
    };

    let backend_name = match backend {
        SpawnBackend::ClaudeCli { model } => format!("claude:{model}"),
        SpawnBackend::CopilotCli { model } => {
            format!("copilot:{}", model.as_deref().unwrap_or("default"))
        }
        SpawnBackend::Script { command, .. } => format!("script:{command}"),
    };

    Ok(SpawnedProcess {
        pid: child.id(),
        workspace: workspace.to_path_buf(),
        backend: backend_name,
    })
}

/// Validate a worktree after creation: .git exists, branch is correct.
fn validate_worktree(wt_path: &Path, expected_branch: &str) -> RuntimeResult<()> {
    // Check .git file/dir exists (worktrees have a .git file pointing to main repo)
    let git_marker = wt_path.join(".git");
    if !git_marker.exists() {
        return Err(RuntimeError::Internal(format!(
            "worktree validation failed: .git missing at {}",
            wt_path.display()
        )));
    }
    // Verify git recognizes this as a valid worktree
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(wt_path)
        .output()
        .map_err(|e| RuntimeError::Internal(format!("git rev-parse: {e}")))?;
    if !status.status.success() {
        return Err(RuntimeError::Internal(format!(
            "worktree validation failed: git does not recognize {}",
            wt_path.display()
        )));
    }
    // Verify we're on the expected branch
    let branch_out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(wt_path)
        .output()
        .map_err(|e| RuntimeError::Internal(format!("git branch check: {e}")))?;
    let actual = String::from_utf8_lossy(&branch_out.stdout)
        .trim()
        .to_string();
    if actual != expected_branch {
        return Err(RuntimeError::Internal(format!(
            "worktree on wrong branch: expected '{expected_branch}', got '{actual}'"
        )));
    }
    tracing::debug!(path = %wt_path.display(), branch = expected_branch, "worktree validated");
    Ok(())
}

/// Cleanup: remove worktree after agent completes.
pub fn cleanup_worktree(repo_root: &Path, wt_path: &Path) -> RuntimeResult<()> {
    let output = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(wt_path)
        .current_dir(repo_root)
        .output()
        .map_err(|e| RuntimeError::Internal(format!("git worktree remove: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("worktree cleanup failed: {stderr}");
    }
    Ok(())
}

/// Patch .mcp.json in worktree to use CEO profile for delegated agents.
/// CEO = single `cvg_ceo` tool that routes via LLM, saving ~10K tokens/session.
fn write_mcp_ceo_profile(workspace: &Path) -> RuntimeResult<()> {
    let mcp_path = workspace.join(".mcp.json");
    if !mcp_path.exists() {
        tracing::debug!("no .mcp.json in worktree, skipping CEO profile patch");
        return Ok(());
    }
    let raw = std::fs::read_to_string(&mcp_path)
        .map_err(|e| RuntimeError::Internal(format!("read .mcp.json: {e}")))?;
    let patched = raw
        .replace("\"compact\"", "\"ceo\"")
        .replace("\"full\"", "\"ceo\"");
    std::fs::write(&mcp_path, patched)
        .map_err(|e| RuntimeError::Internal(format!("write .mcp.json: {e}")))?;
    Ok(())
}

// Re-export backend_for_tier from spawn_backend module
pub use crate::spawn_backend::backend_for_tier;

use crate::spawn_backend::{resolve_claude_path, resolve_gh_path};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn write_instructions_creates_task_and_init() {
        let rel_base = std::path::Path::new(".");
        let path = write_instructions(rel_base, "Test task").unwrap();
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Test task"));
        assert!(content.contains("ONE feature at a time"));
        let init = rel_base.join("init.sh");
        assert!(init.exists());
        // Cleanup
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(init);
    }

    #[test]
    fn write_mcp_ceo_profile_patches_compact_to_ceo() {
        let tmp = tempfile::tempdir().unwrap();
        let mcp = tmp.path().join(".mcp.json");
        fs::write(&mcp, r#"{"env":{"CONVERGIO_MCP_PROFILE":"compact"}}"#).unwrap();
        write_mcp_ceo_profile(tmp.path()).unwrap();
        let out = fs::read_to_string(&mcp).unwrap();
        assert!(out.contains("\"ceo\"") && !out.contains("\"compact\""));
    }

    #[test]
    fn write_mcp_ceo_profile_noop_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        write_mcp_ceo_profile(tmp.path()).unwrap(); // should not panic
        assert!(!tmp.path().join(".mcp.json").exists());
    }
}
