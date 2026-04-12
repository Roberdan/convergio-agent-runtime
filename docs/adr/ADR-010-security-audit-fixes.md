# ADR-010: Security Audit Fixes for Agent Runtime

**Status:** Accepted
**Date:** 2025-07-25
**Author:** Security Audit (Copilot)

## Context

The `convergio-agent-runtime` crate (6687 LOC) manages agent lifecycle:
spawning processes, creating worktrees, executing code, and managing
concurrent agents. Given it executes arbitrary code via CLI backends,
a security audit was performed covering OWASP categories.

## Findings and Decisions

### 1. Path Traversal in Context Enrichment (CRITICAL → FIXED)

**File:** `context_enrichment.rs`
**Issue:** `enrich_with_file_context` joined user-influenced paths from
instruction text with workspace path without validating containment.
An attacker crafting instructions with `../../etc/passwd` could read
arbitrary files.
**Fix:** Added `canonicalize()` + `starts_with()` containment check,
plus `..` component rejection before path resolution.

### 2. Sentinel File Path Injection (CRITICAL → FIXED)

**File:** `adaptation.rs`
**Issue:** `write_sentinel` and `clear_sentinel` accepted arbitrary
`name` parameters, enabling write/delete of files outside
`.convergio/` via path traversal (e.g., `../../../etc/cron.d/evil`).
**Fix:** Whitelisted sentinel names to `["STOP", "PRIORITY_CHANGE",
"CHECKPOINT_READY"]`. Any other name is rejected before filesystem ops.

### 3. Missing Input Validation on Spawn Endpoint (HIGH → FIXED)

**File:** `spawn_routes.rs`
**Issue:** `agent_name`, `org_id`, `repo_override`, and
`instruction_file` were accepted without validation. These flow into
branch names, environment variables, and filesystem paths.
**Fix:**
- `agent_name`/`org_id`: 1-64 chars, alphanumeric + dash + underscore
- `repo_override`: must be absolute, no `..` components
- `instruction_file`: simple filename, no path separators

### 4. Predictable /tmp Workspace Path (MEDIUM → FIXED)

**File:** `allocator.rs`
**Issue:** Initial workspace path used `/tmp/cvg-agent-{uuid8}` which
is world-writable and predictable (symlink attack vector).
**Fix:** Changed to `pending-{uuid8}` placeholder — the actual worktree
path is set by `spawn_routes` after `create_worktree()`.

### 5. Codegraph File Count / Traversal (MEDIUM → FIXED)

**File:** `codegraph_routes.rs`
**Issue:** `POST /api/codegraph/expand` accepted unlimited files array
with no traversal check.
**Fix:** Added max 50 files limit and `..` rejection.

### 6. Unsafe Blocks Documentation (LOW → FIXED)

**File:** `monitor_helpers.rs`
**Issue:** `kill_process` and `try_reap` use `unsafe` libc calls
without SAFETY comments explaining the invariants and risks.
**Fix:** Added comprehensive `# Safety` documentation and inline
SAFETY comments explaining PID recycling risk and mitigations.

### 7. No Authentication on Endpoints (ACKNOWLEDGED — NOT FIXED)

**All route files**
**Issue:** Zero endpoints have auth middleware. Any network-reachable
client can spawn/kill agents, read/write context, etc.
**Decision:** The runtime is designed to run on localhost only (bound
to 127.0.0.1:8420 by the daemon). Network exposure would require
adding auth middleware at the daemon level. This is tracked as a
future enhancement but not blocking for the current deployment model.

### 8. Script Backend Command Injection (ACKNOWLEDGED)

**File:** `spawner.rs`
**Issue:** `SpawnBackend::Script { command, args }` passes user-controlled
values to `Command::new()`. However, this backend is only reachable via
`backend_for_tier()` which maps tiers to Claude/Copilot CLI — never
to arbitrary scripts via the HTTP API.
**Decision:** The current code path is safe. If Script backend is ever
exposed via API, a command whitelist must be added.

### 9. SQL Injection (CLEAN)

All queries use `rusqlite::params!` parameterized statements. No string
interpolation in SQL was found. No fix needed.

### 10. Race Conditions (ACCEPTABLE)

Rate limiter uses `tokio::sync::Mutex` which is correct for async.
The TOCTOU between rate check and spawn is minimal and bounded by
the `MAX_WORKTREES` quota in `spawner.rs`.

## Consequences

- Path traversal vectors eliminated in context enrichment and sentinels
- Input validation prevents injection via agent names and paths
- Unsafe blocks properly documented per Rust conventions
- No breaking API changes — all fixes are additive validation
