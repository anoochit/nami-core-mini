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
   - **Web Search & Data:** Google search integration (`google_search`), date/time utilities, and weather forecasting.
   - **Model Context Protocol (MCP):** Full support for both Stdio subprocess servers and remote streamable HTTP MCP endpoints (`src/mcp.rs`).

3. **Persistent Memory & Session Management (`src/memory.rs`, `src/workspace.rs`)**
   - SQLite-backed state storage (`adk-session`, `adk-memory`) for multi-turn conversations and long-term agent recall.
   - Dual-mode operation: Interactive CLI chat (`chat`, `ask`) and a high-performance HTTP server (`serve`) built with Axum (`src/server.rs`).

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
├── nami.json                 # Agent configuration, sub-agents, tools, and MCP servers
├── src/
│   ├── main.rs               # CLI entrypoint and subcommand dispatcher
│   ├── agent.rs              # Agent builder, model setup, and plugin configuration
│   ├── config.rs             # Configuration structs and validation logic
│   ├── server.rs             # Axum HTTP server and API endpoints
│   ├── memory.rs             # SQLite memory and session adapters
│   ├── workspace.rs          # Security guard for file operations
│   ├── compaction.rs         # Context window compaction logic
│   ├── mcp.rs                # Stdio and HTTP MCP server managers
│   ├── media/                # Multimedia generation, assembly, and manifests
│   └── tools/                # Built-in toolsets (filesystem, search, media, etc.)
├── book/                     # Comprehensive conceptual guide & architecture chapters
└── wiki/                     # Spikes, implementation plans, and feature comparisons
```

---

## Getting Started

### Prerequisites
- **Rust 2024 edition** (nightly/stable toolchain matching Cargo.toml)
- **Google Gemini API Key** (configured in `.env` or environment variables)

### Configuration
Edit `nami.json` to configure model names, system instructions, enabled tools, MCP servers, and media generation options.

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
