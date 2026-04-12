//! Helper functions for spawn_monitor: file reading, path resolution,
//! cross-platform process management.

use std::path::Path;

/// Kill a process by PID.
/// # Safety
/// Uses libc::kill which requires a valid PID. If the PID has been recycled
/// by the OS, a different process may receive the signal. The caller must
/// ensure the PID is still associated with the intended agent process.
#[cfg(unix)]
pub fn kill_process(pid: u32) {
    // SAFETY: libc::kill sends SIGTERM to the specified PID. We accept
    // the inherent TOCTOU risk of PID recycling — the reaper polls frequently
    // (every 5s) which minimizes the window for PID reuse.
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
pub fn kill_process(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
}

/// Try to reap the process. Returns Some(exit_code) if exited.
/// # Safety
/// Uses libc::waitpid and libc::kill(pid, 0) for process status checks.
/// Same PID-recycling caveat as `kill_process`.
#[cfg(unix)]
pub fn try_reap(pid: u32) -> Option<i32> {
    let mut status: i32 = 0;
    // SAFETY: WNOHANG makes this non-blocking. We only call this for PIDs
    // we spawned, and the polling interval (5s) limits recycling risk.
    let r = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
    if r > 0 {
        if libc::WIFEXITED(status) {
            return Some(libc::WEXITSTATUS(status));
        }
        return Some(-1);
    }
    if r < 0 {
        // SAFETY: kill(pid, 0) checks process existence without sending a signal.
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        if !alive {
            return Some(-1);
        }
    }
    None
}

#[cfg(not(unix))]
pub fn try_reap(pid: u32) -> Option<i32> {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    if s.contains(&pid.to_string()) {
        None
    } else {
        Some(-1)
    }
}

/// Read the last N lines from a file. Returns empty string if file is missing.
pub fn read_tail(path: &Path, lines: usize) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .rev()
        .take(lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolve gh CLI path (launchd has minimal PATH).
pub fn resolve_gh_path() -> String {
    if let Ok(p) = std::env::var("CONVERGIO_GH_BIN") {
        return p;
    }
    let candidates = ["/opt/homebrew/bin/gh", "/usr/local/bin/gh"];
    for c in &candidates {
        if Path::new(c).exists() {
            return c.to_string();
        }
    }
    "gh".into()
}

/// Check if an agent's plan uses single_branch execution mode.
/// In single_branch mode: no push/PR per wave, no worktree cleanup between waves.
pub fn is_plan_single_branch(pool: &convergio_db::pool::ConnPool, agent_id: &str) -> bool {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let plan_id: Option<i64> = conn
        .query_row(
            "SELECT plan_id FROM art_agents WHERE id = ?1",
            rusqlite::params![agent_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let Some(plan_id) = plan_id else {
        return false;
    };
    let mode: Option<String> = conn
        .query_row(
            "SELECT execution_mode FROM plans WHERE id = ?1",
            rusqlite::params![plan_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    mode.as_deref() == Some("single_branch")
}
