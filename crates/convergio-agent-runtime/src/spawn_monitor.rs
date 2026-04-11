//! Spawn monitor — watches agent processes, handles completion.
//! On exit: check commits → push/PR → update plan task status → auto-respawn.

use std::path::Path;
use std::process::Command;

use convergio_db::pool::ConnPool;
use tokio::task::JoinHandle;

/// Track a spawned agent process. Returns a JoinHandle for the monitor task.
pub fn monitor_agent(
    pool: ConnPool,
    agent_id: String,
    pid: u32,
    workspace: String,
    _repo_root: String,
    agent_name: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Sentinel watcher: kill agent on STOP signal
        let ws_clone = workspace.clone();
        let sentinel_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                if let Some(s) = crate::adaptation::check_sentinel(&ws_clone) {
                    if s == "STOP" {
                        tracing::info!(pid, "STOP sentinel detected, killing agent");
                        kill_process(pid);
                        break;
                    }
                }
            }
        });
        // Wait for process to exit (poll every 5s)
        let exited = wait_for_exit(pid).await;
        sentinel_handle.abort();
        tracing::info!(
            agent_id = agent_id.as_str(),
            pid,
            exit = exited,
            "agent process exited"
        );

        let ws = Path::new(&workspace);

        // Check if agent committed
        let committed = has_new_commits(ws);
        let branch = get_branch_name(ws);

        let log_content = std::fs::read_to_string(ws.join("agent.log")).unwrap_or_default();
        let err_tail = read_tail(&ws.join("agent.err"), 10);

        if let Some(usage) = crate::token_parser::parse_agent_log(&log_content) {
            crate::token_parser::record_to_db(&pool, &agent_id, &usage);
            tracing::info!(
                agent_id = agent_id.as_str(),
                backend = usage.backend.as_str(),
                model = usage.model.as_str(),
                input = usage.input_tokens,
                output = usage.output_tokens,
                cost = usage.cost_usd,
                turns = usage.num_turns,
                "token usage recorded"
            );
        }

        let stage = if committed { "stopped" } else { "failed" };
        if let Ok(conn) = pool.get() {
            let _ = conn.execute(
                "UPDATE art_agents SET stage = ?1, updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![stage, agent_id],
            );
        }

        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".into());
        if let Err(e) = convergio_ipc::agents::unregister(&pool, &agent_name, &hostname) {
            tracing::debug!(agent = agent_name.as_str(), "IPC deregister: {e}");
        }

        // Push + PR unless single_branch mode (push happens once at plan completion)
        if committed {
            let is_single_branch = crate::monitor_helpers::is_plan_single_branch(&pool, &agent_id);
            if is_single_branch {
                tracing::info!(
                    agent_id = agent_id.as_str(),
                    "single_branch: commit local, no push/PR per wave"
                );
            } else if let Some(ref branch) = branch {
                let existing_pr = find_pr_for_branch(ws, branch);
                if let Some(pr_number) = existing_pr {
                    tracing::info!(
                        agent_id = agent_id.as_str(),
                        branch,
                        pr_number,
                        "PR already exists for branch — skipping push/create"
                    );
                } else {
                    let push_ok = push_branch(ws, branch);
                    if push_ok {
                        let pr_url = create_pr(ws, branch);
                        tracing::info!(
                            agent_id = agent_id.as_str(),
                            branch,
                            pr = pr_url.as_deref().unwrap_or("failed"),
                            "agent work pushed"
                        );
                    }
                }
            }
            update_plan_task(&pool, &agent_id, "submitted");
        } else {
            update_plan_task(&pool, &agent_id, "failed");
        }

        if stage == "stopped" || stage == "failed" {
            let repo = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".into());
            let daemon = std::env::var("CONVERGIO_DAEMON_URL")
                .unwrap_or_else(|_| "http://localhost:8420".into());
            match crate::respawn::try_respawn(&pool, &agent_id, &daemon, &repo) {
                Ok(Some(new_id)) => {
                    tracing::info!(
                        agent_id = agent_id.as_str(),
                        new = new_id.as_str(),
                        "continuation spawned"
                    );
                }
                Ok(None) => tracing::debug!(agent_id = agent_id.as_str(), "no respawn needed"),
                Err(e) => {
                    tracing::warn!(agent_id = agent_id.as_str(), error = %e, "respawn failed")
                }
            }
        }

        // Log summary
        if !err_tail.is_empty() {
            tracing::warn!(agent_id = agent_id.as_str(), "agent stderr:\n{err_tail}");
        }
        tracing::info!(
            agent_id = agent_id.as_str(),
            committed,
            stage,
            "agent monitor complete"
        );

        // Schedule worktree cleanup after 5 min (single_branch: skip — reused by next wave)
        let skip_cleanup = crate::monitor_helpers::is_plan_single_branch(&pool, &agent_id);
        let ws_cleanup = workspace.clone();
        tokio::spawn(async move {
            if skip_cleanup {
                tracing::debug!("single_branch mode: skipping worktree cleanup for {ws_cleanup}");
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            let ws_path = std::path::Path::new(&ws_cleanup);
            if ws_path.exists() {
                if let Some(repo_root) = convergio_workspace::reaper::find_repo_root(ws_path) {
                    if let Err(e) =
                        convergio_workspace::reaper::remove_worktree(&repo_root, ws_path)
                    {
                        tracing::warn!("post-agent cleanup failed for {ws_cleanup}: {e}");
                    } else {
                        tracing::info!("post-agent cleanup: removed worktree {ws_cleanup}");
                    }
                }
            }
        });
    })
}

/// Poll until process exits. Returns exit code or -1.
async fn wait_for_exit(pid: u32) -> i32 {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        if let Some(code) = crate::monitor_helpers::try_reap(pid) {
            return code;
        }
    }
}

fn kill_process(pid: u32) {
    crate::monitor_helpers::kill_process(pid);
}

/// Check if the agent made commits beyond the base.
/// Tries `origin/main..HEAD` first; falls back to `main..HEAD` for offline
/// or pre-push environments where origin/main is not yet available.
fn has_new_commits(workspace: &Path) -> bool {
    for base in &["origin/main..HEAD", "main..HEAD"] {
        if let Ok(o) = Command::new("git")
            .args(["log", "--oneline", base])
            .current_dir(workspace)
            .output()
        {
            if o.status.success() {
                return !String::from_utf8_lossy(&o.stdout).trim().is_empty();
            }
        }
    }
    false
}

/// Check if a PR already exists for this branch via `gh pr list`.
/// Returns the PR number if found, None if not found or gh unavailable.
fn find_pr_for_branch(workspace: &Path, branch: &str) -> Option<u64> {
    let gh = resolve_gh_path();
    let out = Command::new(&gh)
        .args([
            "pr", "list", "--head", branch, "--json", "number", "--limit", "1",
        ])
        .current_dir(workspace)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let parsed: serde_json::Value = // gh returns [{"number": N}] or []
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).ok()?;
    parsed.as_array()?.first()?.get("number")?.as_u64()
}

/// Get current branch name.
fn get_branch_name(workspace: &Path) -> Option<String> {
    Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(workspace)
        .output()
        .ok()
        .and_then(|o| {
            let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        })
}

/// Push the current branch to origin.
fn push_branch(workspace: &Path, branch: &str) -> bool {
    let output = Command::new("git")
        .args(["push", "-u", "origin", branch])
        .current_dir(workspace)
        .output();
    match output {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            tracing::warn!("git push failed: {stderr}");
            false
        }
        Err(e) => {
            tracing::warn!("git push error: {e}");
            false
        }
    }
}

/// Create a PR via gh CLI.
fn create_pr(workspace: &Path, branch: &str) -> Option<String> {
    let gh = resolve_gh_path();
    let title = format!("feat: agent work on {branch}");
    let output = Command::new(&gh)
        .args([
            "pr",
            "create",
            "--base",
            "main",
            "--title",
            &title,
            "--body",
            "Produced by convergio agent.\n\n🤖 Generated with convergio daemon",
        ])
        .current_dir(workspace)
        .output()
        .ok()?;
    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(url)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("gh pr create failed: {stderr}");
        None
    }
}

fn read_tail(path: &Path, lines: usize) -> String {
    crate::monitor_helpers::read_tail(path, lines)
}

fn resolve_gh_path() -> String {
    crate::monitor_helpers::resolve_gh_path()
}

#[path = "plan_task_update.rs"]
mod plan_task_update;
use plan_task_update::update_plan_task;
