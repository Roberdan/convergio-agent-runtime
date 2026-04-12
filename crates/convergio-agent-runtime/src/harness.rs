//! Harness engineering templates — baseline test, TASK.md header, init.sh.
//!
//! Implements the Anthropic harness pattern (adapted for Convergio):
//! - Baseline test before every session
//! - One feature at a time
//! - Agent reads from DB, not static files
//! - Thor uses a separate model for evaluation

use crate::types::{RuntimeError, RuntimeResult};
use std::path::Path;

/// Write init.sh baseline test script to the worktree.
/// Agent MUST run this before starting work. If it fails, fix first.
pub fn write_baseline_script(workspace: &Path) -> RuntimeResult<()> {
    convergio_types::platform_paths::validate_path_components(workspace)
        .map_err(RuntimeError::Internal)?;
    let path = workspace.join("init.sh");
    std::fs::write(&path, BASELINE_SCRIPT)
        .map_err(|e| RuntimeError::Internal(format!("write init.sh: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)
            .map_err(|e| RuntimeError::Internal(format!("metadata init.sh: {e}")))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| RuntimeError::Internal(format!("chmod init.sh: {e}")))?;
    }
    Ok(())
}

/// Resolve the model for Thor evaluation (separate from coding agent).
/// Defaults to Sonnet — different from the typical Opus coding tier.
pub fn thor_model() -> String {
    std::env::var("CONVERGIO_THOR_MODEL").unwrap_or_else(|_| "claude-sonnet-4-6".to_string())
}

/// Header prepended to every TASK.md — delegation rules + harness rules.
pub const DELEGATION_HEADER: &str = "\
# REGOLE OPERATIVE (leggere PRIMA di iniziare)

## STEP 0: Read rules + baseline
1. Read `agents/common.md` + `agents/executor.md` for all rules.
2. Run `bash init.sh` — if it fails, fix the baseline first.

## ONE feature at a time
Work on ONE feature per session. Test it. Commit it.
One feature = one commit = one verifiable result.

## MCP tools
Use `cvg_help` for the full workflow and `cvg_complete_task` to close the task.

## PR body (NON-NEGOTIABLE — 5 mandatory sections)
```
## Problem
## Why
## What changed
## Validation
## Impact
```

## Before commit
- cargo fmt + clippy -D warnings
- Isolated worktree, max 300 lines/file
- Conventional commit message";

const BASELINE_SCRIPT: &str = r#"#!/bin/bash
# Baseline test — run BEFORE starting any work.
# If this fails, fix the baseline first. Do NOT start new work on a broken base.
set -e

echo "=== Baseline: cargo check ==="
cd daemon && cargo check --workspace

echo "=== Baseline: cargo test ==="
cargo test --workspace

echo "=== Baseline: daemon health ==="
curl -sf http://localhost:8420/api/health || echo "WARN: daemon not reachable (ok if not running locally)"

echo "=== Baseline PASSED ==="
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn write_baseline_creates_init_sh() {
        let _tmp = tempfile::tempdir().unwrap();
        // Create a relative path by using current dir as base
        let rel_base = std::path::Path::new(".");
        write_baseline_script(rel_base).unwrap();
        let init = rel_base.join("init.sh");
        assert!(init.exists());
        let content = fs::read_to_string(&init).unwrap();
        assert!(content.contains("cargo check"));
        assert!(content.contains("cargo test"));
        assert!(content.contains("Baseline PASSED"));
        // Cleanup
        let _ = fs::remove_file(init);
    }

    #[test]
    fn delegation_header_has_harness_rules() {
        assert!(DELEGATION_HEADER.contains("init.sh"));
        assert!(DELEGATION_HEADER.contains("ONE feature at a time"));
        assert!(DELEGATION_HEADER.contains("cvg_complete_task"));
        assert!(DELEGATION_HEADER.contains("## Problem"));
        assert!(DELEGATION_HEADER.contains("## Validation"));
        // Header must stay lean — agents waste turns reading verbose boilerplate
        let line_count = DELEGATION_HEADER.lines().count();
        assert!(
            line_count <= 30,
            "DELEGATION_HEADER is {line_count} lines — must be <= 30"
        );
    }

    #[test]
    fn thor_model_defaults_to_sonnet() {
        if std::env::var("CONVERGIO_THOR_MODEL").is_err() {
            assert!(thor_model().contains("sonnet"));
        }
    }
}
