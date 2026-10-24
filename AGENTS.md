# AGENTS.md — Development Guide for AI Agents & Contributors

Welcome to **Nami Mini** (`nami-core-mini`), an advanced AI assistant and autonomous agent framework built in Rust on Google's **Agent Development Kit (`adk-rust` 2.3.0)** and **Axum 0.8**.

This document serves as the authoritative operational guide for AI agents (and human developers) working in or modifying this codebase.

---

## 1. Core Engineering Principles

- **Single Binary Ethos:** One compiled process (`nami`), one local SQLite database (`nami.db`), and one HTTP port. No background microservices, Redis queues, or dynamic C-style plugin libraries (`dlopen`).
- **Zero `unsafe`:** The entire codebase in `src/` contains zero `unsafe` blocks.
- **Zero Clippy Warnings:** `cargo clippy` must pass with zero warnings across all modules.
- **Colocated Testing:** Unit and integration tests live directly inside the file they test within `#[cfg(test)]` modules (no separate `tests/` folder).
- **Plain Text Response Policy:** Nami responds strictly in plain text paragraphs without markdown formatting (no bold, italics, markdown headings, code blocks, backticks, or tables).

---

## 2. Repository Structure & Module Map

The core implementation consists of **14 modules across 26 `.rs` files** under `src/`:

```text
nami-core-mini/
├── Cargo.toml                # Dependencies (adk-rust 2.3.0, axum 0.8, tokio, etc.)
├── nami.json                 # Primary configuration (system instructions, sub-agents, MCP, shortcuts)
├── .env.example              # Template environment variables (GOOGLE_API_KEY, PORT, RUST_LOG)
├── .skills/                  # Auto-injected skills (code-reviewer, multimodal-pipeline)
├── wiki/                     # Plain-text Markdown knowledge base indexed into SQLite on boot
├── book/                     # 20-chapter conceptual guide and architecture documentation
├── .github/workflows/        # GitHub Actions (release.yml multi-arch build automation)
└── src/
    ├── main.rs               # Entrypoint & CLI subcommand dispatcher (serve, chat, ask)
    ├── agent.rs              # Agent builder, root + 5 specialists, adk-retry-reflect integration
    ├── api_client.rs         # SSE-speaking HTTP client powering CLI surfaces
    ├── cli.rs                # Interactive REPL, prompt parsing, custom shortcuts, SSE rendering
    ├── config.rs             # NamiConfig deserialization, defaults, and validation
    ├── server.rs             # Axum HTTP server wrapper (/api, /ui/), health checks, graceful shutdown
    ├── memory.rs             # NamiMemory SQLite session service and wiki Markdown indexer
    ├── workspace.rs          # WorkspaceGuard path confinement and read-only sandboxing
    ├── compaction.rs         # Conversation history compaction trigger configuration
    ├── mcp.rs                # Stdio subprocess and remote HTTP MCP toolset managers
    ├── grill.rs              # Interactive /grill REPL command for architectural idea refining
    ├── plan.rs               # Interactive /plan REPL command for task decomposition into PLAN.md
    ├── goal.rs               # Interactive /goal REPL command for autonomous Ralph Wiggum loops
    ├── media/                # Multimodal generation tools (image, TTS, music, video, FFmpeg)
    └── tools/                # Built-in toolsets (filesystem, google_search, weather, datetime)
```

---

## 3. Essential Commands & Quality Gates

Run these commands to verify code changes before committing:

```bash
# 1. Fast compile check
cargo check

# 2. Lint check (must pass with 0 warnings)
cargo clippy

# 3. Test suite execution (111+ tests)
cargo test

# 4. Running Nami locally
cargo run -- chat                       # Start interactive CLI REPL
cargo run -- ask "Explain Nami"          # Run a single query and exit
cargo run -- serve --port 8080          # Run HTTP API & Web UI on port 8080
```

---

## 4. Configuration Standards (`nami.json`)

All runtime identity and agent capabilities are declared in `nami.json`:

- **`system_instruction`:** Defines root agent persona and mandates plain text output without markdown.
- **`commands`:** Configurable custom slash command shortcuts in the REPL (e.g., `/review`, `/summarize`). Supports optional `{args}` placeholders or context appending.
- **`agents`:** Array of specialist sub-agents (`researcher`, `coder`, `reviewer`, `tester`, `media`) invoked via `transfer_to_agent`.
- **`workspace`:** `allowed_paths` array and `read_only` boolean enforced by `WorkspaceGuard`.
- **`media`:** `enabled` flag controlling access to Gemini image, TTS, music, and video tools.
- **`mcpServers`:** Stdio command subprocesses or streamable HTTP remote MCP toolsets.

---

## 5. Coding Conventions for Agents

1. **Output & Printing:**
   - `println!` and `eprintln!` are restricted to `src/cli.rs` and `src/main.rs`.
   - All other modules must log via `tracing` (`tracing::info!`, `tracing::warn!`, `tracing::error!`).
2. **Error Handling:**
   - Use `anyhow::Result<T>` with actionable context error strings (`anyhow::bail!`, `.context(...)`).
   - Do not invent custom `Error` enums for crate-local tools.
3. **Bilingual Comments:**
   - **Thai** comments carry architectural reasoning, invariants, and SDK constraints.
   - **English** carries log outputs, user-facing error messages, and API wire strings.
4. **CLI Stream Emojis:**
   - Stream rendering in `src/cli.rs` uses standardized section headers and status emojis:
     - `🤔 thinking:` for model reasoning phase.
     - `💬 answer:` for model text response.
     - `🛠️ tool:` for tool execution sections.
     - `🔧 call: {name}({args})` for tool function calls.
     - `✅ {name}` / `❌ {name}` for tool result status.
     - `💭 thought` printed on a new line before execution metrics.

---

## 6. Automation & REPL Slash Commands

Interactive REPL commands in `nami chat`:
- `/new` — Start a fresh conversation session.
- `/session` — Display current server URL, app name, and session ID.
- `/grill` — Start/compile an architectural requirements interview into `GRILL.md`.
- `/plan` — Generate a structured implementation plan in `PLAN.md`.
- `/goal` — Run autonomous goal loop with deterministic shell or natural language verification.
- Custom shortcuts defined in `nami.json` under `"commands"` (e.g., `/review`, `/summarize`).
