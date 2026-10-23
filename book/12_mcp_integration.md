# Chapter 12: The MCP Integration — Giving Nami Hands

I cannot write a client for every protocol under the sun. The Model Context Protocol solves
that by making the server side somebody else's problem, and my side of the bargain is one
config key and a startup sequence that refuses to fall over.

## 1. One Key, Two Kinds of Server

Both flavours live under `mcpServers`, in the same `mcp.json` shape:

```json
"mcpServers": {
  "echo-server": {
    "command": "npx",
    "args": ["-y", "mcp-echo-server"],
    "disabled": false
  },
  "remote-echo": {
    "url": "https://echo.example.com/mcp",
    "disabled": true
  }
}
```

The split is decided by which field is present, and the type system enforces it:

```rust
// src/config.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerEntry {
    /// stdio server — managed by `McpServerManager` (requires `command`)
    Stdio(McpServerConfig),
    /// remote server — connects directly over streamable HTTP (requires `url`)
    Remote(RemoteMcpServerConfig),
}
```

`untagged` means the deserializer tries `Stdio` first and falls back to `Remote`. `stdio_servers()` and `remote_servers()` then filter the map back apart by the same discriminator, and each half goes down its own code path.

## 2. The stdio Path

`load_mcp_manager` builds a manager, starts every child process, and hands the handle back
to `main` so it can shut them down cleanly later:

```rust
// src/mcp.rs
let manager = match McpServerManager::from_json(&json) {
    Ok(manager) => Arc::new(manager),
    Err(err) => {
        tracing::warn!("Unable to load MCP configuration: {}", err);
        return None;
    }
};
```

Three things in that file are worth more than their line count.

**The Windows fix.** On Windows, `tokio::process::Command` does not resolve `npx` — it needs
`npx.cmd`. So before starting anything:

```rust
// src/mcp.rs
if cfg!(windows) {
    for (id, config) in manager.all_configs().await {
        let needs_extension = !config.command.contains('.')
            && matches!(config.command.as_str(), "npx" | "npm" | "pnpm" | "yarn");
        if needs_extension {
            let mut replacement = config.clone();
            replacement.command = format!("{}.cmd", config.command);
            ...
        }
    }
}
```

Without that, every `npx`-based server fails to spawn on Windows with a "program not found"
that looks like a missing install. It is a `cfg!` guard, so it costs nothing elsewhere.

**Start-all is a report, not a gate.** Each server's outcome is logged individually and
nothing throws:

```rust
let outcomes = manager.start_all().await;
for (id, outcome) in outcomes {
    match outcome {
        Ok(_) => tracing::info!("Started MCP Server [{}] successfully", id),
        Err(err) => tracing::warn!("MCP Server [{}] failed to start: {}", id, err),
    }
}
```

One dead MCP server degrades my tool count. It does not take the assistant down with it.

**Monitoring stays on.** `manager.start_monitoring()` enables the auto-restart behaviour, so
a child that dies comes back rather than silently disappearing from my toolset.

## 3. The Remote Path

Remote servers do not go through the manager at all. They connect directly over streamable
HTTP, one connection each:

```rust
// src/mcp.rs
async fn connect_remote_server(id: &str, server: &RemoteMcpServerConfig)
    -> anyhow::Result<McpToolset<()>>
{
    let mut builder = McpHttpClientBuilder::new(server.url.clone());
    for (name, value) in &server.headers {
        builder = builder.header(name.clone(), value.clone());
    }
    if let Some(token) = &server.bearer_token {
        builder = builder.with_auth(McpAuth::bearer(token.clone()));
    }
    Ok(builder.connect().await?.with_name(id))
}
```

Auth comes in two grades: `bearer_token` for the common `Authorization: Bearer …` case, and
an arbitrary `headers` map for everything else (`X-Api-Key`, custom tenancy headers). Use the
simple one when it fits — the complex one is there so you never have to hand-roll a token
string into a header name.

The `disabled` flag is checked before the connection is even attempted, which is how you keep
a declared-but-undeployed server from producing a startup warning every boot.

## 4. The Failure Philosophy

Both paths share one rule: **a failed server is logged and skipped, never fatal.**

```
tracing::warn!("MCP remote server [{id}] failed to connect ({}): {err}", server.url);
```

A remote MCP server is a network dependency, and network dependencies fail at 3 a.m. If my
startup aborted because of one, I would be un-startable for reasons unrelated to the task
you woke me for. Instead I come up with fewer tools and tell you in the logs.

The consequence you should plan for: **tool availability is not constant.** If a workflow
depends on an MCP tool, check the startup logs before debugging the model. "The model keeps
saying it cannot find that tool" is very often "that server never connected".

## 5. Shutdown Discipline

The manager handle is threaded from `build_nami_agent` back to `main` for one reason:

```rust
// src/agent.rs
// Keep the handle so `main` can call `shutdown()` — otherwise the server would still be
// running at drop time
let mcp_manager_handle = mcp_manager.clone();
```

Child processes left running at exit are the classic orphaned-MCP-server bug. Every command
arm in `main` — `serve`, `chat`, `ask` — calls `mcp.shutdown()` on the way out.

## 6. Scope

- MCP servers attach to the **root agent only** (Chapter 9). Sub-agents do not see them.
- No sampling, no resources, no elicitation — tools only. `McpToolset` can do more; I do not
  wire it up.

Next: how the same server serves me and you.
