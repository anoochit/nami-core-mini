# Nami Mini

Nami Mini is an advanced AI assistant and autonomous agent framework built in Rust, powered by the **Agent Development Kit (`adk-rust`)** and Google's Gemini models. It combines a robust single-binary architecture with hierarchical multi-agent delegation, Model Context Protocol (MCP) support, SQLite-backed persistent memory and sessions, context compaction, workspace security sandboxing, and multimodal media generation (image, text-to-speech, background music, and video synthesis).

---

## Key Features

1. **Modular Agent Architecture (`src/agent.rs`, `src/config.rs`)**
   - Hierarchical multi-agent setup: A primary root agent (`nami-mini`) with specialized sub-agents (`researcher`, `coder`, `reviewer`, `tester`, `media`).
   - Automatic skill injection (`adk-skill`) and tool delegation via `transfer_to_agent`.
   - Retry & Reflection plugin (`adk-retry-reflect`) allowing agents to self-correct upon tool execution failures.

2. **Rich Tooling & Ecosystem Integration (`src/tools/`)**
   - **Devtools & Filesystem:** Sandboxed file reading/writing, directory traversal, and command execution governed by `WorkspaceGuard`.
   - **Web Search & Data:** Google search integration (`google_search`), Web HTML plain-text extractor (`web_fetch`), date/time utilities, and weather forecasting.
   - **Local Knowledge Retrieval:** `search_wiki` queries the `wiki/` Markdown knowledge base indexed into SQLite long-term memory — no API key and no network required.
   - **Model Context Protocol (MCP):** Full support for both Stdio subprocess servers and remote streamable HTTP MCP endpoints (`src/mcp.rs`).

3. **Persistent Memory & Background Server (`src/memory.rs`, `src/server.rs`)**
   - SQLite-backed state storage (`adk-session`, `adk-memory`) for multi-turn conversations and long-term agent recall.
   - Long-term memory is driven by `wiki/`: Markdown documents are re-indexed at every startup and recalled at runtime through the `search_wiki` tool.
   - High-performance HTTP server (`serve`) built with Axum (`src/server.rs`) exposing SSE streams, Web UI (`/ui/`), durable background runs (`/api/runs`), and scheduled cron jobs (`/api/cron`).
   - CLI modes (`chat`, `ask`) supporting configurable output visibility (`show_tool_call`, `show_tool_result`, `show_thinking`) and clean answer-only ask responses.

4. **Advanced Context Compaction (`src/compaction.rs`)**
   - Post-run conversation history compaction to maintain efficient context windows over long interaction sessions.

5. **Multimodal Media Pipeline (`src/media/`, `src/tools/media.rs`)**
   - Generates rich multimedia content from stories or topics using specialized Gemini models:
     - **Images** (`generate_image`)
     - **Voiceover Audio / TTS** (`generate_tts`)
     - **Background Music** (`generate_music` via Lyria)
     - **Video Clips** (`generate_video` via Gemini Interactions API)
     - **Audio/Video Assembly** (`assemble_audio_video`)

---

## Project Structure

```text
├── Cargo.toml                # Rust dependencies and workspace configuration
├── nami.json                 # Agent configuration, sub-agents, tools, MCP servers, and CLI toggles
├── .skills/                  # Automatic skill definitions (code-reviewer, multimodal-pipeline)
├── src/
│   ├── main.rs               # CLI entrypoint and subcommand dispatcher
│   ├── agent.rs              # Agent builder, model setup, and plugin configuration
│   ├── api_client.rs         # SSE-speaking HTTP client for CLI surfaces
│   ├── cli.rs                # Interactive REPL, prompt parsing, stream rendering, and REPL commands (/skills, /stats)
│   ├── config.rs             # Configuration structs and validation logic
│   ├── server.rs             # Axum HTTP server, static UI routes, background runs, and cron jobs
│   ├── memory.rs             # SQLite memory adapter and wiki Markdown knowledge indexer
│   ├── workspace.rs          # Security guard for file operations and sandboxing
│   ├── compaction.rs         # Context window compaction logic
│   ├── mcp.rs                # Stdio subprocess and remote HTTP MCP server managers
│   ├── grill.rs              # Interactive /grill REPL command for idea refining
│   ├── plan.rs               # Interactive /plan REPL command for task decomposition
│   ├── goal.rs               # Interactive /goal REPL command for autonomous loops
│   ├── media/                # Multimedia generation, assembly, and manifests
│   └── tools/                # Built-in toolsets (filesystem, search, fetch, weather, datetime, wiki)
├── book/                     # Comprehensive conceptual guide & architecture chapters
└── wiki/                     # Spikes, implementation plans, and feature comparisons
```

---

## Getting Started

### Prerequisites

- **Rust 2024 edition** (nightly/stable toolchain matching Cargo.toml)
- **Google Gemini API Key** (configured in `.env` or environment variables)

### Configuration

Edit `nami.json` to configure model names, system instructions, enabled tools, MCP servers, CLI display toggles (`show_tool_call`, `show_tool_result`, `show_thinking`), and media generation options.

### Running Nami Mini

- **Start interactive chat:**

  ```bash
  cargo run -- chat
  ```

- **Ask a single question:**

  ```bash
  cargo run -- ask "Explain how Nami Mini works"
  ```

- **Start the HTTP server:**

  ```bash
  cargo run -- serve --port 8080
  ```

### In-Chat Commands (REPL)

Interactive sessions in `nami chat` support built-in slash commands:

| Command | Effect |
|---------|--------|
| `/new` | Start a new conversation session (resets `/stats` counters) |
| `/session` | Show current server URL, app name, and session ID |
| `/stats` | Show token usage for the current session (prompt, output, thinking, total) |
| `/skills` | List auto-loaded skills (the same skill index the agents load from `.skills/`) |
| `/help` | List all commands, including custom shortcuts |
| `/grill` | Interview to clarify requirements and write `GRILL.md` |
| `/plan` | Generate a structured implementation plan (`PLAN.md`) |
| `/goal` | Run autonomous Ralph Wiggum development loop |
| `exit` / `quit` | Leave the REPL |

Custom prompt shortcuts (e.g. `/review`, `/summarize`, `/wiki`) are declared in `nami.json` under `"commands"`, with optional `{args}` substitution, and expand into model prompts before the skill matcher and tools run.
