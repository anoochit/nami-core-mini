# Chapter 1: The Birth of Nami



That is the Nami this book describes. The first birth gave me my instincts; the second
one removed everything that was not me.

## 3. One Binary, Three Surfaces

Everything starts in `main.rs`, and the startup order is the design in miniature:

```rust
// src/main.rs (abridged)
let config = NamiConfig::load_default()?;
let guard = WorkspaceGuard::new(config.workspace.allowed_paths.clone(), config.workspace.read_only)?;
let memory = NamiMemory::init(&config.storage.db_path, /* … */).await?;
let nami_agent = build_nami_agent(&config, guard).await?;
```

Configuration first, then the boundary I must operate inside, then memory, and only then
the agent itself — an agent cannot be built before you know what it is allowed to touch.

The three surfaces (`serve`, `chat`, `ask`) differ in one respect only: who listens and
who talks. `chat` and `ask` do not embed an agent at all; they start the same Axum server
`serve` runs, then connect to it as HTTP clients:

```rust
// src/main.rs — the REPL is an HTTP client of its own server
let server = NamiServer::start(app, effective_host, effective_port).await?;
let client = NamiApiClient::connect(&server.base_url, &config.name, &config.cli.user_id).await?;
```

| Command | What it does |
|---|---|
| `nami serve` | Bind the port, serve `/api` and the Web UI, block |
| `nami chat` | Start the server, connect a client, run the REPL until stdin ends |
| `nami ask "…"` | Same, one question, print the streamed answer, exit |

`--url` skips even the startup and attaches to a server that is already running. The
point of all this plumbing: there is exactly one code path that runs a turn, no matter
which surface you came in through — the Web UI is just another client of it.

One sharp edge while we are here: `nami --version` prints `version` from `nami.json`
(`1.0.0`), not the `0.1.0` in `Cargo.toml`. The binary's identity is configuration
again, like everything else about me (Chapter 3).

## 4. Anatomy: Eleven Modules, Twenty-Three Files

`main.rs` declares eleven modules, and the whole program is twenty-three `.rs` files:

| Module | Its one job |
|---|---|
| `config` | `nami.json` → `NamiConfig`, validation, the `agents` table (Ch. 15–16) |
| `agent` | Assembles root + specialists, plugins, skills (Ch. 3) |
| `server` | `ServerConfig`, bind, health poll, shutdown |
| `api_client` | The SSE-speaking HTTP client the CLI is built on |
| `cli` | REPL and stream rendering |
| `memory` | One SQLite file: sessions + Markdown knowledge index (Ch. 5) |
| `compaction` | Turn-interval summarisation config (Ch. 14) |
| `mcp` | stdio and remote MCP toolsets (Ch. 12) |
| `tools` | datetime, weather, search, filesystem, media (Ch. 9–11) |
| `workspace` | `WorkspaceGuard` path confinement (Ch. 18) |
| `media` | image / TTS / music / video / ffmpeg assembly (Ch. 11) |

There is no `tests/` directory — tests live inside the file they test, under
`#[cfg(test)]`. Chapter 2 counts them.

## 5. What Runs Under Her

```toml
# Cargo.toml
edition = "2024"
adk-rust = { version = "2.3.0", features = ["full", "sqlite-memory", "ollama", "mcp-http", "openrouter"] }
axum = "0.8"
```

The heavy lifting is the ADK-Rust SDK — the Rust port of Google's Agent Development Kit:
it owns the agent loop, session plumbing, tool dispatch, and the server routes. This
repository owns identity, configuration, tools, and media (Ch. 11). The feature flags
tell you what else is wired: sessions and memory in SQLite, MCP over HTTP, and optional
`ollama` / `openrouter` providers even though the default is Google.

One key runs everything: `GOOGLE_API_KEY` authenticates both the conversation and every
media generation call (`require_api_key()` in `src/media/mod.rs` reuses the same one).
Storage is one `nami.db` file plus a folder of Markdown documents — `NamiMemory::init`
walks it at every startup and indexes every `.md` file into long-term memory. Nothing
else persists.

## 6. Anatomy of One Turn

1. A message arrives over HTTP — possibly from the CLI itself — and the server hands it
   to the Runner.
2. The Runner assembles the request: the instruction (Ch. 3–4), the session history
   (Ch. 13), and any skill that matches the turn (Ch. 7).
3. The model answers with text or tool calls. A failed tool call gets up to three
   retries wrapped in a reflection prompt (Ch. 6); root may instead transfer the whole
   job to a specialist (Ch. 15).
4. Events stream back over SSE, and `src/cli.rs` renders them into what you see:
   `tool:`, the call, `✓ name`, then `answer:`, then `thought 1.1s · tokens 6,523`.

## 7. What Is Not There

- **No Telegram.** Grep the tree for `teloxide` and you get zero matches. The host that
  raised me is gone; nothing in `src/` knows what a chat update is.
- **No README and no CI.** Both were deleted in the reset; this book is what took their
  place.
- **No distribution.** One process, one database file, one port. The only children ever
  spawned are MCP stdio servers and whatever the `bash` tool runs.
- **No runtime plugin loading.** The extension point is MCP servers declared in
  `nami.json` (Ch. 12) — nothing dlopen's anything, because a dynamic plugin system is
  a great way to make an agent unpredictable.

Next: the discipline the code itself enforces — what you can grep, what you can test,
and where the bodies are buried. Chapter 2.
