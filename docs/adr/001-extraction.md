# ADR-001: Extract convergio-agent-runtime from monorepo

Status: accepted
Date: YYYY-MM-DD

Context: Monorepo too large for agents. This crate has clear boundaries and minimal coupling.

Decision:
- Extract into own repo with convergio-sdk as dependency
- Follow convergio-crate-template standard
- CI, release-please, cargo-deny, adversarial tests as applicable

Consequences:
- Agents work on ~XXXX LOC instead of ~103K LOC
- CI runs in ~15-20s instead of ~1m14s
- SDK version updates come via dependabot
