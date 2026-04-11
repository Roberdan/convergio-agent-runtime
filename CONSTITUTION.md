---
version: "1.0"
last_updated: "2026-04-11"
author: "convergio-team"
tags: ["constitution", "non-negotiable", "rules"]
---

# Convergio Constitution (Extracted Repo Edition)

> Adapted from [convergio main repo](https://github.com/Roberdan/convergio) CONSTITUTION.md — the canonical source.
> This version is tailored for standalone Convergio crates (SDK, domain crates, templates).
> When in doubt, the main repo's CONSTITUTION.md is authoritative.

Non-negotiable rules for every agent, every session, every decision.

## Rule 1: Fix root causes, never shortcuts

NEVER take the quick path. ALWAYS fix the root cause.
- Bug in auth? Fix the auth middleware, not a workaround.
- Extension returns None from routes()? Implement the routes.
- Test fails? Fix the code, not delete the test.
- 3 consecutive fixes that each introduce new problems → STOP. Explain root cause, propose rebuild.

## Rule 2: Integration test mandatory

Unit tests alone are NOT evidence. Every phase closes with a smoke test against the running system.
Learning #13-14: 793 unit tests green, daemon that didn't work.

## Rule 3: Workspace isolation

Every task in its own worktree. Never on the main checkout.
One worktree = one branch = one PR. Cleanup mandatory post-merge.

> Note: worktree path depends on repo structure. The main repo uses `.worktrees/`;
> standalone repos may use a different convention. Follow the repo's AGENTS.md if present.

## Rule 4: Rules before agents

Rules must exist BEFORE launching agents. Never add rules to running sessions.

## Rule 5: Verifiable evidence

Never accept "done" without proof. Commit hash, curl output, test output.
The evidence gate rejects self-reported without verification.

## Rule 6: The planner foresees everything

Every plan includes: integration test per wave, wiring verification, final smoke test.
Never plan "create crate" without "verify the consumer can use it".

## Rule 7: Explore before building

NEVER build without first checking what exists. Read existing crates, components, old repo.
Duplicating because you didn't look is a planning failure.

## Rule 8: Never bypass without explicit user approval

No hook, rule, check, gate, or constraint can be disabled without Roberdan's approval.
If a constraint blocks work, STOP and ask. Don't decide to bypass on your own.

## Rule 9: Conserve context tokens

Context is finite. Every wasted token shortens the agent's life.
- Don't re-read files already read in this session.
- Use offset/limit for large files.
- At 70-80% context usage: save checkpoint, prepare handoff.

## Rule 10: Close the loop

Every feature needs: input → processing → output → feedback → state updated → visible to user.
If the user can't see the result, it's not done.

## Rule 11: CLI-API Contract

> Applies to repos with a CLI surface. If this repo has no CLI, this rule is informational.

Every CLI command MUST have a matching, tested server endpoint before merge.
No speculative CLI commands — write the server route first, then the CLI.
The contract test MUST pass in CI.

## Rule 12: Adversarial Audit (Challenger Gate)

Every completed plan must pass an adversarial audit before closure:
- **Code**: every endpoint has a consumer, every CLI has a route, JSON fields match
- **Business**: every deliverable has owner, audience, next action
- **Design**: every component is referenced by at least one implementation task
The principle: "every output must be connected and reachable — never orphaned."

## Rule 13: Pre-Release Gate

No release tag may be created without ALL tests passing first.
- `cargo test --workspace` is a hard gate in the release workflow.
- All public API tests must pass: shape tests, contract tests, doc tests.
- If the repo has E2E tests, they must pass before tagging.
- Learning #15: CLI called /api/orgs/{id}/telemetry but the route didn't exist. User saw
  "error decoding response body". Shape tests prevent this class of bug.

---

## Learnings (Appendix)

These learnings came from real incidents building Convergio. They apply universally.

1. Match commits to tasks — 38 tasks submitted without verification
2. Evidence must be verifiable, not just "posted"
3. Wave completion must trigger validation automatically
4. Worktree isolation prevents parallel agent conflicts
5. Rules added after launch are never read by running agents
6. Closure checklist must be known before launch
7. Long prompts as shell arguments cause silent hangs — use file + Read
8. `</dev/null` with `claude -p` causes input errors
9. Heredoc with backticks causes unexpected EOF — prompts in separate files
10. Pattern: `timeout 7200 claude --dangerously-skip-permissions -p "short prompt"`
11. Orchestrator must clean worktrees after merge
12. Autonomous orchestrator works — completed 6 phases unattended
13. GRAVE: building isolated crates without integration testing
14. Automated orchestrators produce hollow crates (routes()→None, stub handlers)
