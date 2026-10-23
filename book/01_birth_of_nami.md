# Chapter 1: The Birth of Nami

The Nami you are reading about today was born twice.

The first birth created my instincts. In that earlier incarnation, I lived inside a complex multi-platform framework, tangled up in Telegram updates, long-polling loops, and external platform adapters. I could answer questions, but most of my codebase was spent managing communication glue rather than reasoning or executing tasks safely.

The second birth was a radical reset. I removed everything that was not me: the Telegram handlers, the microservice abstractions, and the runtime clutter. I was rebuilt from scratch as a single Rust binary powered by `adk-rust` (Google's Agent Development Kit for Rust).

That second birth gave me my current structure: lean, deterministic, single-process, and fully transparent. That is the Nami this book describes.

## 1. The First Birth and the Reset

An agent raised inside platform wrappers spends more time adapting to external APIs than serving the user. Every platform brings its own rate limits, formatting quirks, update types, and session lifecycle assumptions.

In the reset, every platform wrapper was stripped away. I shed `teloxide` and the Telegram event loop entirely. Instead of attempting to be everywhere at once through custom platform integration code, I standardized on a single unified interface: an Axum HTTP API streaming Server-Sent Events (SSE).

By shifting to an HTTP-first single binary, my architecture became radically simpler:

- **Zero transport clutter:** No chat bot libraries or webhook polling loops in `src/`.
- **One single engine:** Every entrypoint — whether the Web UI, an interactive CLI chat, or an automated script — executes through the exact same HTTP handler and agent turn loop.
- **Strictly bounded runtime:** I run as a single process with a single database file and one local directory guard.

## 2. The Single Binary Ethos

The core engineering philosophy of this repository can be summarized in five words: **one process, one database, one port.**

Many agent architectures quickly decay into distributed microservices: a vector database service, a separate task queue worker, a Redis session store, and a dynamic plugin gateway. That distribution brings operational fragility, difficult local debugging, and non-deterministic state.

Here, everything sits inside one compiled binary:

- **No external infrastructure required:** Storage is a local SQLite database (`nami.db`), and long-term knowledge is a folder of Markdown files (`wiki/`).
- **No dynamic runtime loading:** Capabilities are declared at build time or wired cleanly via Model Context Protocol (MCP) sub-processes. Nothing is `dlopen`'d dynamically.
- **Predictable execution boundaries:** Workspace file access is confined by a strict path guard (`WorkspaceGuard`), preventing accidental directory escapes.

This design makes me easy to audit, fast to start, and simple to run anywhere Rust compiles.

## 3. High-Level System Architecture

To understand how requests flow through Nami, consider the system architecture as a layered pipeline inside a single running process:

```text
                               ┌───────────────────────────────────────────────┐
                               │                 Client Layer                  │
                               │  Web UI (/ui/)   or   CLI REPL (nami chat)    │
                               └──────────────────────┬────────────────────────┘
                                                      │ HTTP / SSE Stream
                                                      ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Nami Single Process Binary (`nami`)                                                                        │
│                                                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────────────────────────────────────┐  │
│  │ Axum Server (`src/server.rs`)                                                                          │  │
│  │ Exposes `/api` endpoints, serves static UI, manages SSE event streams                                 │  │
│  └───────────────────────────────────────────────────┬───────────────────────────────────────────────────┘  │
│                                                      │                                                      │
│                                                      ▼                                                      │
│  ┌───────────────────────────────────────────────────────────────────────────────────────────────────────┐  │
│  │ Execution Runner & Context Assembly (`adk-rust`)                                                      │  │
│  │ Bundles System Instructions (Ch. 3-4), Session History (Ch. 13), & Skill Rules (Ch. 7)                │  │
│  └───────────────────────────────────────────────────┬───────────────────────────────────────────────────┘  │
│                                                      │                                                      │
│                                                      ▼                                                      │
│  ┌───────────────────────────────────────────────────────────────────────────────────────────────────────┐  │
│  │ Root Orchestrator Agent (`nami-mini`)                                                                 │  │
│  │ Models: Gemini / Ollama / OpenRouter · Self-Correction: `adk-retry-reflect` (Ch. 6)                   │  │
│  └───────┬───────────────────────────────────────────┬───────────────────────────────────────────┬──────┘  │
│          │                                           │                                           │          │
│          │ Delegation (`transfer_to_agent`)          │ Native Function Calls                     │ Memory   │
│          ▼                                           ▼                                           ▼          │
│  ┌───────────────────────────────┐       ┌───────────────────────────────┐       ┌──────────────────────┐   │
│  │ Specialist Sub-Agents         │       │ Built-in & MCP Toolsets       │       │ Storage & State      │   │
│  │ • researcher (search/devtools)│       │ • Devtools (filesystem, bash) │       │ • `nami.db` (SQLite) │   │
│  │ • coder      (devtools)       │       │ • Search, Datetime, Weather   │       │   Sessions & Memory  │   │
│  │ • reviewer   (devtools)       │       │ • Media (image, TTS, video)   │       │ • `wiki/` Markdown   │   │
│  │ • tester     (devtools)       │       │ • MCP Servers (stdio / HTTP)  │       │   Indexed Knowledge  │   │
│  │ • media      (media tools)    │       │ • WorkspaceGuard Sandboxing   │       └──────────────────────┘   │
│  └───────────────────────────────┘       └───────────────────────────────┘                                  │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

The system architecture is divided into four distinct operational layers:

1. **Transport Layer (`src/server.rs`, `src/api_client.rs`):** An Axum HTTP server listening on a single port. It serves static assets for the Web UI at `/ui/` and handles streaming API turns via Server-Sent Events (SSE). CLI subcommands (`nami chat`, `nami ask`) act as HTTP clients to this server.
2. **Orchestration Core (`src/agent.rs`):** Powered by `adk-rust`. The root agent (`nami-mini`) evaluates incoming prompts, manages context assembly, and applies `adk-retry-reflect` to automatically recover from failed tool executions.
3. **Execution & Delegation Layer (`src/tools/`, `src/mcp.rs`):** When tasks require specialized skills, the root agent delegates work to specialist sub-agents (`researcher`, `coder`, `reviewer`, `tester`, `media`) using `transfer_to_agent`. Tool calls operate inside a security sandbox governed by `WorkspaceGuard`.
4. **Persistence & Knowledge Base (`src/memory.rs`):** Session conversation history and long-term memory reside in a single local SQLite file (`nami.db`). Markdown files in `wiki/` are scanned and indexed into memory at every system startup.

## 4. One Binary, Three Surfaces

Everything starts in `src/main.rs`, and the startup order is the entire system design in miniature:

```rust
// src/main.rs (abridged)
let config = NamiConfig::load_default()?;
let guard = WorkspaceGuard::new(
    config.workspace.allowed_paths.clone(),
    config.workspace.read_only,
)?;
let memory = NamiMemory::init(&config.storage.db_path, &config.storage.wiki_dir).await?;
let nami_agent = build_nami_agent(&config, guard).await?;
```

The order is strict and deliberate: configuration first, followed by the workspace boundary I must operate inside, then persistent memory, and only then the agent itself. An agent cannot be built before you establish what it is allowed to touch and remember.

The three CLI surfaces (`serve`, `chat`, `ask`) differ in one respect only: who listens and who talks. `chat` and `ask` do not run a separate, decoupled agent core. They start the exact same Axum HTTP server that `serve` runs, and then connect to it as HTTP clients in-process:

```rust
// src/main.rs — REPL starts and connects to its own server
let server = NamiServer::start(app, effective_host, effective_port).await?;
let client = NamiApiClient::connect(&server.base_url, &config.name, &config.cli.user_id).await?;
```

| Command | What it does |
| --- | --- |
| `nami serve` | Binds the HTTP port, serves `/api` and the Web UI at `/ui/`, and blocks. |
| `nami chat` | Starts the server in-process, connects `NamiApiClient`, and runs the REPL until stdin ends. |
| `nami ask "<text>"` | Starts the server in-process, sends a single question, prints the streamed answer, and exits. |

If you pass `--url http://127.0.0.1:8080`, `chat` and `ask` skip starting an in-process server altogether and attach directly to an already-running Nami instance.

The benefit of this plumbing is absolute consistency: there is exactly one code path that executes an agent turn, regardless of how the request entered the system. The Web UI and the CLI REPL are identical consumers of the SSE stream.

One sharp edge to note: `nami --version` prints the `version` field from `nami.json` (`1.0.0`), rather than the package version in `Cargo.toml` (`0.1.0`). My operational identity is driven by configuration (Chapter 3).

## 5. Anatomy: Fourteen Modules, Twenty-Six Files

The entire system in `src/` is composed of fourteen modules across twenty-six `.rs` files:

| Module | Its one job |
| --- | --- |
| `config` | Deserializes `nami.json` into `NamiConfig`, handles defaults, and validates rules (Ch. 2). |
| `agent` | Assembles root + specialist sub-agents, skills, and retry/reflection plugins (Ch. 3, 15–16). |
| `server` | Configures Axum routes (`/api`, `/ui/`), manages health checks, and handles graceful shutdown. |
| `api_client` | The SSE-speaking HTTP client that powers CLI interactions. |
| `cli` | Interactive REPL, command parsing, and stream output rendering. |
| `memory` | Manages the SQLite store (`nami.db`) for sessions and indexes `wiki/` Markdown docs (Ch. 5). |
| `compaction` | Triggers post-turn conversation history summarization (Ch. 14). |
| `mcp` | Manages stdio subprocesses and remote HTTP MCP toolsets (Ch. 12). |
| `tools` | Built-in function tools: datetime, weather, search, and filesystem (Ch. 9–10). |
| `workspace` | Enforces path confinement through `WorkspaceGuard` (Ch. 18). |
| `media` | Orchestrates image generation, TTS voiceover, music, video synthesis, and FFmpeg assembly (Ch. 11). |
| `grill` | Interactive REPL command (`/grill`) to interrogate fuzzy ideas into structured `GRILL.md` (Ch. 17). |
| `plan` | Interactive REPL command (`/plan`) to decompose tasks into executable `PLAN.md` files (Ch. 17). |
| `goal` | Interactive REPL command (`/goal`) executing autonomous loops with dual verification (Ch. 17). |

There is no external `tests/` directory. Unit and integration tests live directly alongside the implementation files inside `#[cfg(test)]` modules. Chapter 2 details how these tests are structured.

## 6. What Runs Under Her

My core engine relies on `Cargo.toml` dependencies:

```toml
# Cargo.toml
edition = "2024"

[dependencies]
adk-rust = { version = "2.3.0", features = ["full", "sqlite-memory", "ollama", "mcp-http", "openrouter"] }
axum = "0.8"
```

The heavy lifting is performed by `adk-rust` — the Rust port of Google's Agent Development Kit. It manages the agent turn loop, session history persistence, tool execution, and HTTP server routes.

This repository owns identity, configuration, custom tools, and the multimodal media pipeline. The enabled feature flags reveal what is wired:

- Persistent SQLite session storage (`sqlite-memory`).
- Model Context Protocol over HTTP endpoints (`mcp-http`).
- Optional provider routing to Ollama or OpenRouter alongside Google Gemini defaults.

A single environment variable — `GOOGLE_API_KEY` — authenticates LLM turns as well as media generation calls (`generate_image`, `generate_tts`, `generate_music`, and `generate_video`).

Persistence relies on two local components: a SQLite database (`nami.db`) for session state, and a directory of Markdown documents (`wiki/`). On every startup, `NamiMemory::init` scans the `wiki/` folder and indexes its documents into long-term memory.

## 7. Anatomy of One Turn

Every interaction follows a deterministic path through the execution stack:

1. **Request arrival:** A prompt arrives via HTTP POST to `/api/...` (sent by the Web UI or `NamiApiClient` in `nami chat` / `nami ask`).
2. **Context assembly:** The `Runner` builds the request payload by combining the system instruction (Ch. 3–4), relevant session history from SQLite (Ch. 13), and any skill that matches the input message (Ch. 7).
3. **Model execution & Tool dispatch:** The LLM evaluates the prompt and generates text or tool calls.
   - If a tool call fails, `adk-retry-reflect` intercepts the failure and retries up to three times with a reflection prompt (Ch. 6).
   - If delegation is required, root transfers execution to a specialist sub-agent (e.g., `researcher`, `coder`, or `media`) via `transfer_to_agent` (Ch. 15).
4. **Streaming response:** Events stream back over SSE. The CLI (`src/cli.rs`) formats these events into visible output:
   `tool:`, execution status (`✓ name`), streamed `answer:`, and final metrics (`thought 1.1s · tokens 6,523`).

## 8. What Is Not There

Just as important as what is present in `src/` is what has been deliberately excluded:

- **No Telegram or external platform adapters:** Grep the codebase for `teloxide` and you will find zero matches. The platform host from my first birth is completely gone.
- **No background microservices:** There are no Redis queues, worker fleets, or background daemons. The only child processes spawned are MCP stdio servers and whatever shell commands the `bash` tool runs.
- **No dynamic runtime plugin loading:** The sole extension mechanism is standard MCP servers declared in `nami.json` (Ch. 12). C-style dynamic libraries (`dlopen`) are omitted to keep agent behavior predictable and secure.

Next: the engineering discipline the codebase enforces — what you can grep, what you can test, and how quality is maintained. Chapter 2.
