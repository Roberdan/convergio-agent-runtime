//! Context enrichment — extract and inject file contents into agent instructions.
//!
//! Parses instructions for file path references (daemon/crates/X/src/Y.rs),
//! reads those files from the worktree, and prepends them as context.
//! This is the key difference between agents that understand vs agents that guess.

use std::path::Path;
use tracing::info;

const MAX_FILE_CONTEXT: usize = 8000; // chars — keep TASK.md under token limits
const MAX_FILES: usize = 6;

/// Extract file paths from instructions and inject their content.
/// Validates all paths stay within the workspace to prevent path traversal.
pub fn enrich_with_file_context(workspace: &Path, instructions: &str) -> String {
    let paths = extract_file_paths(instructions);
    if paths.is_empty() {
        return instructions.to_string();
    }

    // Canonicalize workspace for safe comparison
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => return instructions.to_string(),
    };

    let mut context = String::from("## Files you will be working with\n\n");
    let mut total_chars = 0usize;
    let mut included = 0usize;

    for rel_path in paths.iter().take(MAX_FILES) {
        // Reject paths with traversal components
        if rel_path.contains("..") {
            tracing::warn!(path = rel_path, "skipping path with traversal component");
            continue;
        }
        let full_path = workspace.join(rel_path);
        // Verify resolved path stays within workspace
        let canonical = match full_path.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !canonical.starts_with(&ws_canonical) {
            tracing::warn!(
                path = rel_path,
                "skipping path outside workspace (traversal attempt)"
            );
            continue;
        }
        let content = match std::fs::read_to_string(&canonical) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let truncated = if total_chars + content.len() > MAX_FILE_CONTEXT {
            let remaining = MAX_FILE_CONTEXT.saturating_sub(total_chars);
            if remaining < 200 {
                break;
            }
            // Safe truncation on char boundary to avoid UTF-8 panics
            let safe_end = content
                .char_indices()
                .take_while(|(i, _)| *i < remaining)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            format!("{}...(truncated)", &content[..safe_end])
        } else {
            content.clone()
        };
        total_chars += truncated.len();
        included += 1;
        context.push_str(&format!("### {rel_path}\n```rust\n{truncated}\n```\n\n"));
    }

    if included > 0 {
        info!(
            files = included,
            chars = total_chars,
            "injected file context"
        );
        format!("{context}---\n\n{instructions}")
    } else {
        instructions.to_string()
    }
}

/// Extract Rust file paths from text (daemon/crates/.../src/...rs pattern).
fn extract_file_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for word in text.split_whitespace() {
        let clean = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '/' && c != '.' && c != '-' && c != '_'
        });
        if clean.contains('/')
            && clean.ends_with(".rs")
            && clean.contains("src/")
            && !paths.contains(&clean.to_string())
        {
            paths.push(clean.to_string());
        }
    }
    // Also look for Cargo.toml references
    for word in text.split_whitespace() {
        let clean = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '/' && c != '.' && c != '-' && c != '_'
        });
        if clean.ends_with("Cargo.toml")
            && clean.contains('/')
            && !paths.contains(&clean.to_string())
        {
            paths.push(clean.to_string());
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rust_paths() {
        let text = "Fix daemon/crates/convergio-org/src/routes.rs and check \
                     daemon/crates/convergio-org/Cargo.toml for deps";
        let paths = extract_file_paths(text);
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"daemon/crates/convergio-org/src/routes.rs".to_string()));
        assert!(paths.contains(&"daemon/crates/convergio-org/Cargo.toml".to_string()));
    }

    #[test]
    fn no_paths_in_plain_text() {
        let paths = extract_file_paths("fix the rate limiter bug");
        assert!(paths.is_empty());
    }

    #[test]
    fn deduplicates_paths() {
        let text = "read store.rs at daemon/crates/x/src/store.rs \
                     then modify daemon/crates/x/src/store.rs";
        let paths = extract_file_paths(text);
        assert_eq!(paths.len(), 1);
    }
}
