//! Backend selection and binary resolution for agent spawning.

use super::spawner::SpawnBackend;

/// Resolve the absolute path to the claude binary.
/// launchd services have a minimal PATH — "claude" alone won't be found.
pub fn resolve_claude_path() -> String {
    if let Ok(p) = std::env::var("CONVERGIO_CLAUDE_BIN") {
        return p;
    }
    let candidates = [
        dirs::home_dir()
            .unwrap_or_default()
            .join(".local/bin/claude"),
        dirs::home_dir()
            .unwrap_or_default()
            .join(".claude/bin/claude"),
        std::path::PathBuf::from("/usr/local/bin/claude"),
        std::path::PathBuf::from("/opt/homebrew/bin/claude"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.to_string_lossy().to_string();
        }
    }
    "claude".into()
}

/// Resolve the absolute path to the `gh` binary (for Copilot CLI).
pub fn resolve_gh_path() -> String {
    if let Ok(p) = std::env::var("CONVERGIO_GH_BIN") {
        return p;
    }
    let candidates = [
        std::path::PathBuf::from("/opt/homebrew/bin/gh"),
        std::path::PathBuf::from("/usr/local/bin/gh"),
        dirs::home_dir().unwrap_or_default().join(".local/bin/gh"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.to_string_lossy().to_string();
        }
    }
    "gh".into()
}

/// Choose backend based on model tier.
///
/// POLICY (2026-04-11):
/// - t1: Claude Code + Opus (architecture, security, planning)
/// - t2: Claude Code + Sonnet (mechanical execution with precise instructions)
/// - t3+: Copilot CLI (when permission issues resolved)
///
/// Claude Code for t1/t2: --dangerously-skip-permissions handles
/// file writes reliably. Copilot CLI blocked on write permissions
/// outside worktree. Switch to Copilot when resolved.
pub fn backend_for_tier(tier: &str, model: Option<&str>) -> SpawnBackend {
    match tier {
        "t1" => SpawnBackend::ClaudeCli {
            model: model.unwrap_or("claude-opus-4-6").to_string(),
        },
        "t2" => SpawnBackend::ClaudeCli {
            model: model.unwrap_or("claude-sonnet-4-6").to_string(),
        },
        // WHY: Copilot default model is already Opus — don't pass Claude model names
        _ => SpawnBackend::CopilotCli {
            model: model.map(|m| m.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_routes_to_claude_opus() {
        match backend_for_tier("t1", None) {
            SpawnBackend::ClaudeCli { model } => {
                assert_eq!(model, "claude-opus-4-6");
            }
            other => panic!("expected ClaudeCli for t1, got {other:?}"),
        }
    }

    #[test]
    fn t2_routes_to_claude_sonnet() {
        match backend_for_tier("t2", None) {
            SpawnBackend::ClaudeCli { model } => {
                assert_eq!(model, "claude-sonnet-4-6");
            }
            other => panic!("expected ClaudeCli for t2, got {other:?}"),
        }
    }

    #[test]
    fn t3_routes_to_copilot() {
        match backend_for_tier("t3", None) {
            SpawnBackend::CopilotCli { model } => {
                // WHY: Copilot uses its own default (Opus), no model override needed
                assert_eq!(model, None);
            }
            other => panic!("expected CopilotCli for t3, got {other:?}"),
        }
    }

    #[test]
    fn model_override_works() {
        match backend_for_tier("t1", Some("custom-model")) {
            SpawnBackend::ClaudeCli { model } => {
                assert_eq!(model, "custom-model");
            }
            _ => panic!("expected ClaudeCli"),
        }
    }
}
