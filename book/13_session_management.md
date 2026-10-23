# Chapter 13: Session Management & Resume

The most valuable thing about me in a long conversation is not that I stream fast — it is
that I still know what we said an hour ago. That continuity is a session, and in my build a
session is a row in a SQLite file plus a UUID that travels over HTTP.

## 1. One Binary, Three Surfaces

```
nami chat              # start server in background, run interactive REPL
nami serve             # start the HTTP API server only
nami ask "<text>"      # start server, send one query, print the answer, exit
nami                   # alias for chat
```

The design decision underneath: **the CLI is an HTTP client, not an in-process runner.**
Starting `chat` starts the same Axum server `serve` starts, and the REPL talks to it over
`POST /api/run_sse`. So the terminal, the Web UI at `/ui/`, and any external caller all go
through one wire contract. A mode that talks to a remote server is a flag, not a rewrite.

## 2. The Lifecycle

```
nami chat
 ├── server task        bind → ServerConfig + compaction → create_app → axum::serve
 ├── readiness          poll GET /api/health until 200 (5 s timeout, 50 ms interval)
 ├── REPL loop          stdin line → run_stream → parse SSE → print
 └── exit | Ctrl+C
      oneshot::Sender → graceful shutdown → join task → McpServerManager::shutdown()
```

Two details here are hard-won. The listener binds **before** the task spawns, so a port
collision fails at startup with a clear message instead of halfway through your first query.
And stdin is read on a dedicated thread forwarded over a `tokio::sync::mpsc` channel —
blocking a runtime worker on `stdin` would stall the very server the REPL is waiting on.

```rust
// src/cli.rs
std::thread::spawn(move || {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(line)) = lines.next() {
        if tx.blocking_send(line).is_err() { break; }
    }
});
```

## 3. Sessions Over HTTP

The client creates a session explicitly before its first message:

```rust
// src/api_client.rs
pub async fn new_session(&mut self) -> anyhow::Result<()> {
    let url = format!("{}/api/sessions", self.base_url);
    let body = serde_json::json!({ "appName": self.app_name, "userId": self.user_id });
    ...
}
```

`POST /api/sessions` returns the id, which the client stores and echoes on every
`POST /api/run_sse`. It is optional — `run_sse` will create a missing session for you — but
creating it up front means `/session` can show you a real id and `/new` has exactly one code
path.

In the REPL:

| Command | Effect |
|---------|--------|
| `/new` | `POST /api/sessions`, switch to the fresh id — old context dropped |
| `/session` | Print base URL, app name, session id |
| `/help` | List commands |
| `exit`, `quit` | Shut the server down and leave |

One constraint worth memorising: `appName` must equal the agent's name, which is `name` from
`nami.json`. Rename the agent and any client that cached the old name gets rejected. My client
derives it from config at startup rather than hard-coding it, which is the only reason
renaming is survivable.

## 4. Persistence

Both session and memory services open the same file:

```rust
// src/memory.rs
let session_service = Arc::new(SqliteSessionService::new(&database_url).await?);
session_service.migrate().await?;
```

So `nami.db` is the whole continuity story. Copy it to another machine and I resume there.
Delete it and I meet you for the first time. Migrations run at startup, so an older database
from a previous version is upgraded rather than rejected.

Note what does *not* happen: the REPL never resumes a specific previous session id. It
creates a new one each launch. Resuming means talking to a server that is still running, or
reusing the database across restarts with a client that knows the id.

## 5. Streaming, and Reading the Stream

Every answer arrives as Server-Sent Events. The frames carry `data:` lines only — no
`event:` field — plus `:` comment frames as keep-alives:

```
data: {"author":"nami-mini","invocation_id":"...", ...}

: keep-alive
```

The parser has five rules and no discretion: buffer bytes, split on blank lines, keep partial
frames across chunk boundaries, ignore comments, join multi-line `data:` and deserialize each
as an `adk_rust::Event`. Events then flow through `EventTextDeltas`, the same accumulator the
old in-process REPL used, so rendering stayed byte-identical across the rewrite.

What you see in `chat` is that rendering, sectioned:

```
thinking:
...
tool:
google_search({"query":"..."})
answer:
...
thought 1.8s · tokens 1,240 (prompt 900 · output 160 · thinking 180)
```

That trailing stats line is computed by `TurnTracker`, and it has an edge case worth
respecting: usage is stored per event id, so partial chunks of one model call overwrite
rather than double-count before everything is summed.

`ask` renders the same stream with `verbose: false` — answer text only, no thinking, no tool
calls. Same transport, different amount of courtesy.

## 6. Failure Modes

| Failure | Behaviour |
|---------|-----------|
| Port in use | Fails before the prompt; message names the port and suggests `--port` / `--url` |
| Health never answers | `start` errors after 5 s rather than hanging |
| Server exits mid-session | Next `run_stream` fails to connect; REPL exits non-zero |
| Ctrl+C during a stream | Graceful shutdown cancels the run, then MCP servers are stopped |

## 7. Connecting to a Running Server

```
nami chat --url http://127.0.0.1:8080
```

`--url` skips starting a server entirely and attaches to one already running — which is how
you share a single Nami between a terminal, a web browser, and a script. Port precedence when
you do start one: `--port` beats `PORT` beats `server.port` beats `8080`.

Next: why a long conversation does not eventually blow up.
