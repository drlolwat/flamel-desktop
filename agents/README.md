# Flamel personas and governed skills

Flamel separates reusable workflow knowledge from delegation context.

- Governed skills own triggers, inputs, workflow, safety boundaries, and acceptance evidence. Canonical signed catalog sources live in `runtime-catalog/skills-*.json`, and publication projects the same reviewed version to Codex and Claude.
- Thin personas add only the domain perspective that benefits from an isolated child. They inherit Flamel's runtime, approvals, provider behavior, and tool routing, then activate their named reviewed skill.
- Direct capabilities handle ordinary file and time work. They do not need persona wrappers.
- Project-specific skills belong to their owning project or catalog; this repository publishes only globally reviewed runtime content.

`runtime-catalog/personas.json` is the retained-persona allowlist and complete legacy disposition ledger. The server imports the signed catalog into local SQLite, Chat reads that local copy, and MCP fetches the authenticated localhost persona endpoint once at startup. No persona or governed skill body is compiled into native binaries. Startup removes only unchanged legacy files that exactly match a retired Flamel-managed hash; locally modified files are preserved.

Delegate when isolated expertise or independent parallel work materially improves the outcome. A simple task stays with the current agent. The reusable workflow remains in the skill even when a persona is selected, so a persona never duplicates routing rules or invents authority.

[PERSONA-SKILL-MATRIX.md](PERSONA-SKILL-MATRIX.md) is the traceability record for every legacy personality and workflow owner. [PLATFORM-GUIDELINES.md](PLATFORM-GUIDELINES.md) defines the live-source boundary for platform content. Runtime and tool rules remain in Flamel's shared instructions.
