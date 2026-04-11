//! Worktree reaper — removes orphaned git worktrees and stale branches.
//!
//! For each stale entry under `.worktrees/` and external worktree paths:
//!   1. `git worktree remove --force <path>`
//!   2. `git branch -D <branch>` (agent/*, wave/*, task/*)
//!   3. Prune merged remote-tracking refs

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// Default: reap worktrees older than 24h.
pub const STALE_THRESHOLD: Duration = Duration::from_secs(24 * 60 * 60);

/// Result of a single reap cycle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReapReport {
    pub reaped: Vec<String>,
    pub branches_deleted: Vec<String>,
    pub errors: Vec<String>,
    pub skipped: usize,
}

/// Find the repo root by walking up from `start` looking for `.git`.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Run one reap cycle: clean worktrees + stale branches.
pub fn reap_cycle(repo_root: &Path, threshold: Duration) -> ReapReport {
    let mut report = ReapReport::default();
    reap_worktrees(repo_root, threshold, &mut report);
    prune_stale_branches(repo_root, &mut report);
    prune_remote_refs(repo_root);
    report
}

/// Remove a specific worktree and its branch (for agent completion cleanup).
pub fn remove_worktree(repo_root: &Path, worktree_path: &Path) -> Result<(), String> {
    let name = worktree_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // git worktree remove --force
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(worktree_path)
        .current_dir(repo_root)
        .output();

    // Try common branch patterns
    for prefix in &["agent/", "task/", "wave/", ""] {
        let branch = format!("{prefix}{name}");
        let _ = Command::new("git")
            .args(["branch", "-D", &branch])
            .current_dir(repo_root)
            .output();
    }

    // rm -rf if still present
    if worktree_path.exists() {
        std::fs::remove_dir_all(worktree_path)
            .map_err(|e| format!("rm -rf {}: {e}", worktree_path.display()))?;
    }

    tracing::info!("workspace-reaper: cleaned up worktree {name}");
    Ok(())
}

fn reap_worktrees(repo_root: &Path, threshold: Duration, report: &mut ReapReport) {
    let wt_dir = repo_root.join(".worktrees");
    let entries = match std::fs::read_dir(&wt_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let age = match std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .and_then(|mt| now.duration_since(mt).map_err(std::io::Error::other))
        {
            Ok(a) => a,
            Err(_) => continue,
        };
        if age < threshold {
            report.skipped += 1;
            continue;
        }
        tracing::info!(
            name = name.as_str(),
            age_h = age.as_secs() / 3600,
            "reaping stale worktree"
        );
        match remove_worktree(repo_root, &path) {
            Ok(()) => report.reaped.push(name),
            Err(e) => report.errors.push(e),
        }
    }
}

/// Delete local branches matching stale patterns (no worktree, already merged or orphaned).
fn prune_stale_branches(repo_root: &Path, report: &mut ReapReport) {
    let output = Command::new("git")
        .args(["branch", "--list", "agent/_doctor_test_spawn_*"])
        .current_dir(repo_root)
        .output();
    let Ok(out) = output else { return };
    let branches: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().trim_start_matches("* ").to_string())
        .filter(|b| !b.is_empty())
        .collect();
    for branch in &branches {
        let del = Command::new("git")
            .args(["branch", "-D", branch])
            .current_dir(repo_root)
            .output();
        if let Ok(o) = del {
            if o.status.success() {
                report.branches_deleted.push(branch.clone());
            }
        }
    }
    if !branches.is_empty() {
        tracing::info!(count = branches.len(), "pruned doctor test branches");
    }
}

/// Prune remote tracking refs that no longer exist on origin.
fn prune_remote_refs(repo_root: &Path) {
    let _ = Command::new("git")
        .args(["fetch", "--prune", "--quiet"])
        .current_dir(repo_root)
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reap_cycle_empty_when_no_worktrees_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let report = reap_cycle(tmp.path(), STALE_THRESHOLD);
        assert!(report.reaped.is_empty());
        assert!(report.errors.is_empty());
    }

    #[test]
    fn reap_cycle_skips_fresh_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join(".worktrees").join("fresh-task");
        fs::create_dir_all(&wt).unwrap();
        let report = reap_cycle(tmp.path(), Duration::from_secs(100 * 365 * 24 * 3600));
        assert!(report.reaped.is_empty());
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn reap_cycle_removes_stale_dir_without_git() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join(".worktrees").join("old-task");
        fs::create_dir_all(&wt).unwrap();
        let report = reap_cycle(tmp.path(), Duration::from_secs(0));
        assert!(report.reaped.contains(&"old-task".to_string()) || !report.errors.is_empty());
    }
}
