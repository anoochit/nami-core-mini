# Chapter 9: The Tool Hierarchy

Nobody decides what I can do by reading my prompt. What I can do is decided by what was
attached to which agent at build time, in `build_nami_agent`. This chapter is that function,
read top to bottom.

## 1. The Four Layers

| Layer | Source | Attached to | Chapter |
|-------|--------|-------------|---------|
| Hand-written function tools | `src/tools/` | Root, and to sub-agents by allowlist | 10 |
| Workspace DevTools | `adk-devtools` | Root always; sub-agents via `"devtools"` | — |
| MCP toolsets | `mcpServers` in `nami.json` | Root only | 12 |
| Sub-agents | `agents` in `nami.json` | Root, as transfer targets | 15 |

Everything the model can call is one of those four. There is no dynamic registration and no
runtime tool discovery.

## 2. What the Root Agent Holds

```rust
// src/agent.rs
let mut builder = LlmAgentBuilder::new(&config.name)
    ...
    .with_auto_skills()?
    .enhanced_plugin(retry_reflect_plugin.clone())
    .toolset(Arc::new(dev_toolset))
    .toolset(create_web_fetch_toolset())
    .tool(create_datetime_tool())
    .tool(create_weather_tool())
    .tool(create_search_tool());

for sub_agent in sub_agents {
    builder = builder.sub_agent(sub_agent);
}
if let Some(mcp) = mcp_manager {
    builder = builder.toolset(mcp);
}
for remote in mcp_remote_toolsets {
    builder = builder.toolset(remote);
}
```

Note the order of operations, because it encodes two policies:

- **MCP is root-only.** No sub-agent receives an MCP toolset. If a specialist needs an MCP
  capability, the root has to use it directly, or the sub-agent definition has to change.
- **Sub-agents attach last**, which is also when `adk-agent` injects the
  `transfer_to_agent` tool for me.

## 3. The DevTools Workspace — Two Different Roots

`build_dev_toolset` deserves close reading, because it quietly disagrees with the config:

```rust
// src/agent.rs
fn build_dev_toolset(config: &NamiConfig) -> DevToolset {
    let workspace_root = config
        .workspace
        .allowed_paths
        .first()
        .cloned()
        .unwrap_or_else(|| std::path::PathBuf::from("./"));

    let mut ws = if config.workspace.read_only {
        Workspace::read_only(&workspace_root)
    } else {
        Workspace::new(&workspace_root)
    };
    ws = ws.allow_bash(true);
    DevToolset::new(ws)
}
```

`Workspace::new` takes **one** root — the first entry of `allowed_paths`. The shipped
`nami.json` lists two:

```json
"workspace": { "allowed_paths": ["./", "../shared_docs"], "read_only": false }
```

So the DevTools (`read_file`, `write_file`, `edit_file`, `glob`, `grep`, `bash`, …) operate
inside `./` only. `../shared_docs` is honoured by the separate `WorkspaceGuard` that wraps my
hand-written filesystem tools — see section 5 — but DevTools never see it. Two confinement
mechanisms, two scopes, one config key. Worth knowing before you wonder why a file is
reachable by one tool and not another.

Bash is always on: `allow_bash(true)` is unconditional, and it is not gated by
`read_only`. A read-only workspace stops the file tools from writing; it does not stop
`rm`.

## 4. Sub-Agents Get an Allowlist, Not a Menu

A sub-agent receives exactly the tools named in its `tools` array — nothing inherited, no
defaults:

```rust
// src/agent.rs
for tool in &spec.tools {
    sub_builder = match tool {
        AgentTool::Search => sub_builder.tool(create_search_tool()),
        AgentTool::WebFetch => sub_builder.toolset(create_web_fetch_toolset()),
        AgentTool::Devtools => sub_builder.toolset(Arc::new(build_dev_toolset(config))),
        AgentTool::Filesystem => { /* read_file, write_file, list_files */ }
        AgentTool::Datetime => sub_builder.tool(create_datetime_tool()),
        AgentTool::Weather => sub_builder.tool(create_weather_tool()),
        AgentTool::Media => { /* image, tts, music, video, assemble — Chapter 11 */ }
    };
}
```

An empty `tools` array produces a specialist with **no tools at all** — a pure reasoning
agent. That is occasionally exactly what you want, and it is the cheapest possible
restriction.

One subtlety in that match: `"filesystem"` and `"devtools"` are not the same thing.
`devtools` is the `adk-devtools` suite rooted at the first allowed path; `filesystem` is my
own three-tool set wrapped in `WorkspaceGuard`, which honours *all* of `allowed_paths`. In
this repository every specialist uses `"devtools"`.

## 5. How My Own Tools Are Built

Every hand-written tool is a `FunctionTool` with a schema derived from a Rust type via
`schemars`:

```rust
// src/tools/filesystem.rs
let read_tool = FunctionTool::new(
    "read_file",
    "Read a text file from the workspace and return its contents with line numbers. ...",
    move |_ctx, args| { ... },
)
.with_parameters_schema::<ReadFileParams>();
```

Deriving the schema from `ReadFileParams` rather than hand-writing JSON means the tool
declaration the model sees and the arguments Rust deserialises cannot drift. It also means
doc comments on the struct fields become the argument descriptions the model reads.

## 6. Selection Is the Model's Job

I do not route. There is no dispatcher deciding which tool handles "what's the weather". The
model sees every attached schema and picks. The hierarchy constrains *what exists*; the model
decides *what gets called*; the retry-reflect plugin catches it when that goes wrong
(Chapter 6).

Next: what each of my hand-written tools actually does.
