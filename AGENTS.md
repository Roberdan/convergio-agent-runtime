# Agent Rules — convergio-agent-runtime

Tool-agnostic. Applies to Claude Code, Cursor, Copilot, Windsurf, any AI tool.

## Build & verify (MUST run before every push)

```bash
cargo fmt --all -- --check
RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
cargo deny check
```

Zero tolerance: 0 warnings, 0 skipped tests, 0 TODO without issue link.
If ANY of the above fails, fix it BEFORE pushing. No exceptions.

## CI architecture

CI uses a **reusable workflow** from convergio main repo. DO NOT copy jobs inline.

```
ci.yml → uses: Roberdan/convergio/.github/workflows/reusable-ci-rust.yml@main
```

3 jobs: `check` (fmt+clippy+test+audit+deny) → `quality` (coverage+semver+udeps) → `info` (commitlint+mutants).
Quality only runs if check passes. Info never blocks merge.

## Structure

```
crates/convergio-agent-runtime/
├── src/            — source code (max 250 lines per file)
│   └── lib.rs      — public API, module declarations
├── tests/          — integration + adversarial tests
docs/adr/           — architecture decision records
```

## Rules

- English only (code + docs)
- Max 250 lines per file
- Conventional commits: `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`
- Every extension owns its DB tables via `migrations()`
- Breaking changes = minor version bump (pre-1.0)
- PR requires 5 sections: Problem, Why, What changed, Validation, Impact
- SDK types come from convergio-sdk — never duplicate, always import

## Test requirements

| Type | Location | When required |
|------|----------|---------------|
| Unit | `src/*.rs` `#[cfg(test)]` | Always |
| Integration | `tests/integration_*.rs` | Always |
| Adversarial | `tests/adversarial_*.rs` | If crypto/auth/network |
| Responsible AI | `tests/responsible_ai_*.rs` | If LLM/agent |

Coverage minimum: **70%** (SDK: 80%). Enforced by CI with `cargo tarpaulin --fail-under`.

**IMPORTANT**: Measure coverage BEFORE enabling the gate. Never set `--fail-under N`
if current coverage is below N%. Write tests first, verify locally, then enable.

Test rules:
- Never hardcode counts (`>=` not `==`)
- Never hardcode versions (use `env!("CARGO_PKG_VERSION")`)
- Never skip tests without issue link
- Always run `cargo fmt` after writing tests

## Comments in code

Only 2 types allowed:
- `//!` module doc — 1-2 lines: what it does, who uses it
- `// WHY:` — only when choice is non-obvious from code

## Delegation rules (for orchestrating agents)

When delegating work to sub-agents, EVERY prompt MUST include:

```
After making changes, run:
1. cargo fmt --all
2. cargo clippy --workspace --all-targets --locked
3. cargo test -p <crate> --locked
Do NOT commit — coordinator verifies and commits.
```

Default: t3 (Copilot = free Opus via GitHub). NEVER t2 (Sonnet).
t1 (paid Opus) only for final validation and architecture decisions.

## Workflow

1. Work in worktree: `git worktree add .worktrees/fix-name -b fix/name`
2. Never commit on main checkout
3. Conventional commit, push, create PR with 5 sections
4. Never force-push — new branch + new PR if rebase needed
5. Never merge PRs — leave for batch merge by owner
6. Never close PRs with work — rebase and fix instead

## Do NOT

- Ship without unit + integration tests passing
- Enable coverage gate without measuring first
- Merge PRs without user approval
- Modify SDK types here (changes go in SDK repo)
- Add deps without checking SDK version alignment
- Bypass hooks, tests, or CI gates
- Leave warnings, TODO without issue, or dead code
- Use `cargo install` in CI (use `taiki-e/install-action`)
- Copy CI workflow inline (use reusable from convergio)
- Force-push (new branch + fresh PR instead)
