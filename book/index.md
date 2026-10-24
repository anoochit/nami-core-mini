# Index

### Part 1: The Core (Identity & Context)

*Focus: How Nami thinks and remembers.*

- [Preface: What Is Real in This Edition](00_preface.md)
  - Grounding principles, verification rules, and empirical health gates across the 14-module Rust codebase.
- [Chapter 1: The Birth of Nami](01_birth_of_nami.md)
  - The evolution from platform adapters to a lean, single-process Axum HTTP server powered by `adk-rust` 2.3.0.
- [Chapter 2: AI-Native Engineering](02_ai_native_engineering.md)
  - Zero unsafe, zero clippy warnings, colocated testing, typed configuration in `nami.json`, and bilingual documentation.
- [Chapter 3: The Soul Framework](03_soul_framework.md)
  - System instructions, root agent identity, plain text response policy, and runtime persona enforcement.
- [Chapter 4: Dynamic Context – The Art of Not Being a Blank Slate](04_dynamic_context.md)
  - Prompt templates, instruction providers, automatic skill rules, and session state context assembly.
- [Chapter 5: Long-term Memory – One SQLite File and a Folder of Markdown](05_long_term_memory.md)
  - Session persistence in `nami.db` and automatic booting startup indexing of `wiki/` Markdown knowledge.
- [Chapter 6: The Reflection Service – How I Recover From Being Wrong](06_reflection_service.md)
  - Self-healing execution loops powered by `adk-retry-reflect` tool-level retry and reflection prompts.

### Part 2: The Action Layer (Skills & Tools)

*Focus: What Nami can actually do.*

- [Chapter 7: Skill-Based Execution](07_skill_based_execution.md)
  - Automatic loading, indexing, and runtime injection of skill instructions from `.skills/`.
- [Chapter 8: Building Skills](08_building_skills.md)
  - A practical guide to crafting domain-specific skills with `SKILL.md` frontmatter, assets, and workflows.
- [Chapter 9: The Tool Hierarchy](09_tool_hierarchy.md)
  - The 4-layer capability model: built-in tools, workspace DevTools, MCP toolsets, and specialist sub-agents.
- [Chapter 10: The Digital Swiss Army Knife](10_digital_swiss_army_knife.md)
  - Detailed reference for my 7 built-in tools (`read_file`, `write_file`, `list_files`, `get_datetime`, `get_weather_forecast`, `google_search`, and `web_fetch`).
- [Chapter 11: Multimodal Generation](11_multimodal_generation.md)
  - The media pipeline in `src/media/` orchestrating Gemini image, TTS voiceover, music, video synthesis, and FFmpeg assembly.

### Part 3: Integration & Operations

*Focus: How Nami connects to the world and manages state.*

- [Chapter 12: The MCP Integration — Giving Nami Hands](12_mcp_integration.md)
  - Managing stdio subprocesses and remote HTTP Model Context Protocol (MCP) server integration in `src/mcp.rs`.
- [Chapter 13: Session Management & Resume](13_session_management.md)
  - Session creation, message log persistence in SQLite, and HTTP client connection flows via `NamiApiClient`.
- [Chapter 14: Context Compaction](14_context_compaction.md)
  - Bounding long conversation session contexts via post-turn history compaction and summarization triggers.

### Part 4: Advanced Agency & Governance

*Focus: How Nami acts on her own safely.*

- [Chapter 15: Task Orchestration](15_task_orchestration.md)
  - Sub-agent roster delegation (`researcher`, `coder`, `reviewer`, `tester`, `media`), transfer rules, and shared retry budgets.
- [Chapter 16: Custom Specialist Configuration](16_custom_specialists.md)
  - Declaring custom sub-agents in `nami.json` with model overrides, custom instructions, and tool allowlists.
- [Chapter 17: Automation Loops — Grill, Plan, Goal](17_automation_loops.md)
  - Autonomous engineering workflows (`/grill`, `/plan`, `/goal` Ralph Wiggum loops) plus serve mode background runs (`/api/runs`) and cron scheduling (`/api/cron`).
- [Chapter 18: Security Guardrails](18_security_guardrails.md)
  - Workspace path confinement via `WorkspaceGuard`, read-only sandboxing, shell command safety, and API key security.
- [Chapter 19: Ethical Agency – The Nami Trust Protocol](19_ethical_agency.md)
  - Guidelines for user alignment, accidental data loss prevention, transparent action, and ethical agency limits.
- [Chapter 20: The Future – From Assistant to Architect](20_future_architect.md)
  - The evolutionary roadmap toward architectural autonomy, self-improving workflows, and multi-agent coordination.

---

Every chapter in this book corresponds directly to real code, modules, and execution loops in `src/`.
