# CLAUDE.md — convergio-agent-runtime

Read `AGENTS.md` first — it has all build/test/rules.
Read `CONSTITUTION.md` for non-negotiable rules.
This file adds Claude Code-specific behavior only.

Conversation: **Italian**. Code + docs: **English**.
Co-Authored-By: your model name (e.g. `Claude Opus 4.6`)

## Crate

Agent runtime: allocation, isolation, ownership, scheduling, concurrency, reaper

```
crates/convergio-agent-runtime/src/
├── lib.rs       — public API, module declarations
├── routes.rs    — HTTP routes (axum 0.7, `:id` params)
├── ext.rs       — Extension impl (if applicable)
├── schema.rs    — DB migrations (if applicable)
└── types.rs     — crate-specific types (if applicable)
```

## Workflow

1. Read AGENTS.md for build/test/rules
2. Work in worktree: `git worktree add .worktrees/fix-name -b fix/name`
3. **Before push**: `cargo fmt --all && cargo clippy --workspace && cargo test --workspace --locked`
4. Commit conventional, push, create PR with 5 sections
5. Never merge — owner batch-merges after review

## Delegation

- ALL tasks: delegate to Copilot t3 (free Opus via GitHub). NEVER use Sonnet (t2).
- Keep for yourself ONLY: final validation and architecture decisions
- EVERY delegation prompt MUST include: `cargo fmt --all` + `cargo test` as final steps

## SDK dep

convergio-sdk provides: types, telemetry, security, db.
Never duplicate SDK functionality. Never modify SDK types here.

## CI

Uses reusable workflow from convergio main repo. DO NOT edit CI jobs inline.
If CI fails, check: fmt? clippy? test? coverage? semver? — fix root cause.
