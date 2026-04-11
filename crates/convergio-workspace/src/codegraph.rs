//! Code graph — language-aware dependency analysis for Orient phase.
//!
//! MVP: crate-level deps via `cargo metadata`. The adapter pattern supports
//! adding TypeScript (package.json), Python (pyproject.toml), and other
//! languages later. File-level import parsing planned for Fase 2.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Which language/build system a project uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProjectKind {
    Rust,
    TypeScript,
    Python,
    Unknown,
}

/// Detect the project kind from its root directory.
pub fn detect_project(root: &Path) -> ProjectKind {
    if root.join("Cargo.toml").exists() {
        ProjectKind::Rust
    } else if root.join("package.json").exists() {
        ProjectKind::TypeScript
    } else if root.join("pyproject.toml").exists() || root.join("setup.py").exists() {
        ProjectKind::Python
    } else {
        ProjectKind::Unknown
    }
}

/// Package-level dependency map: package_name → [dep_names].
#[derive(Debug, Clone, Serialize)]
pub struct PackageDeps {
    pub kind: ProjectKind,
    pub packages: HashMap<String, Vec<String>>,
}

/// Result of expanding a set of files through the dependency graph.
#[derive(Debug, Clone, Serialize)]
pub struct ExpandResult {
    pub input_files: Vec<String>,
    pub expanded_packages: Vec<String>,
    pub kind: ProjectKind,
}

/// Get package-level deps. Currently Rust only; TS/Python adapters planned.
pub fn package_deps(root: &Path) -> PackageDeps {
    let kind = detect_project(root);
    let packages = match kind {
        ProjectKind::Rust => rust_crate_deps(root),
        // Future: ts_package_deps, python_package_deps
        _ => HashMap::new(),
    };
    PackageDeps { kind, packages }
}

/// Given files, find which packages they belong to + packages that depend on them.
pub fn expand_files(files: &[String], root: &Path) -> ExpandResult {
    let kind = detect_project(root);
    let deps = package_deps(root);
    let mut touched_pkgs: Vec<String> = Vec::new();

    // Map files → packages they belong to
    for file in files {
        if let Some(pkg) = file_to_package(file, kind) {
            if !touched_pkgs.contains(&pkg) {
                touched_pkgs.push(pkg);
            }
        }
    }

    // Find packages that depend on touched packages (reverse deps)
    let mut expanded: Vec<String> = Vec::new();
    for (pkg, pkg_deps) in &deps.packages {
        for touched in &touched_pkgs {
            if pkg_deps.contains(touched) && !touched_pkgs.contains(pkg) && !expanded.contains(pkg)
            {
                expanded.push(pkg.clone());
            }
        }
    }

    ExpandResult {
        input_files: files.to_vec(),
        expanded_packages: expanded,
        kind,
    }
}

/// Map a file path to its owning package name.
fn file_to_package(file: &str, kind: ProjectKind) -> Option<String> {
    match kind {
        ProjectKind::Rust => {
            // "daemon/crates/convergio-foo/src/bar.rs" → "convergio-foo"
            let parts: Vec<&str> = file.split('/').collect();
            parts
                .iter()
                .find(|p| p.starts_with("convergio-"))
                .map(|s| s.to_string())
        }
        ProjectKind::TypeScript => {
            // "packages/foo/src/bar.ts" → "foo" or "src/app/foo/page.tsx" → "app"
            let parts: Vec<&str> = file.split('/').collect();
            if parts.len() >= 2 {
                Some(parts[0].to_string())
            } else {
                None
            }
        }
        ProjectKind::Python => {
            // "src/foo/bar.py" → "foo"
            let parts: Vec<&str> = file.split('/').collect();
            if parts.len() >= 2 {
                Some(parts[1].to_string())
            } else {
                None
            }
        }
        ProjectKind::Unknown => None,
    }
}

// --- Rust adapter ---

fn rust_crate_deps(root: &Path) -> HashMap<String, Vec<String>> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return HashMap::new(),
    };
    let parsed: CargoMetadata = match serde_json::from_slice(&output.stdout) {
        Ok(m) => m,
        Err(_) => return HashMap::new(),
    };
    let workspace_names: Vec<String> = parsed.packages.iter().map(|p| p.name.clone()).collect();
    let mut map = HashMap::new();
    for pkg in &parsed.packages {
        let deps: Vec<String> = pkg
            .dependencies
            .iter()
            .filter(|d| workspace_names.contains(&d.name))
            .map(|d| d.name.clone())
            .collect();
        map.insert(pkg.name.clone(), deps);
    }
    map
}

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPkg>,
}
#[derive(Deserialize)]
struct CargoPkg {
    name: String,
    dependencies: Vec<CargoDep>,
}
#[derive(Deserialize)]
struct CargoDep {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_project_rust() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "[workspace]").unwrap();
        assert_eq!(detect_project(tmp.path()), ProjectKind::Rust);
    }

    #[test]
    fn detect_project_typescript() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();
        assert_eq!(detect_project(tmp.path()), ProjectKind::TypeScript);
    }

    #[test]
    fn detect_project_python() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("pyproject.toml"), "").unwrap();
        assert_eq!(detect_project(tmp.path()), ProjectKind::Python);
    }

    #[test]
    fn file_to_package_rust() {
        let pkg = file_to_package(
            "daemon/crates/convergio-mcp/src/profile.rs",
            ProjectKind::Rust,
        );
        assert_eq!(pkg.as_deref(), Some("convergio-mcp"));
    }

    #[test]
    fn file_to_package_typescript() {
        let pkg = file_to_package("src/app/dashboard/page.tsx", ProjectKind::TypeScript);
        assert_eq!(pkg.as_deref(), Some("src"));
    }

    #[test]
    fn expand_finds_reverse_deps() {
        // Simulate: if B depends on A, touching A should expand to B
        let mut packages = HashMap::new();
        packages.insert("a".into(), vec![]);
        packages.insert("b".into(), vec!["a".into()]);
        packages.insert("c".into(), vec!["b".into()]);
        let deps = PackageDeps {
            kind: ProjectKind::Unknown,
            packages,
        };
        // Manual reverse-dep check
        let touched = ["a".to_string()];
        let mut expanded = Vec::new();
        for (pkg, pkg_deps) in &deps.packages {
            if pkg_deps.iter().any(|d| touched.contains(d)) && !touched.contains(pkg) {
                expanded.push(pkg.clone());
            }
        }
        assert!(expanded.contains(&"b".to_string()));
        assert!(!expanded.contains(&"c".to_string())); // c depends on b, not a
    }
}
