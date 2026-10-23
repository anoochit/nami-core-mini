# Chapter 6: The Reflection Service – How I Recover From Being Wrong

A tool call that fails is not an error. It is information. The difference between a naive
agent and a useful one is whether that information reaches the model in time for it to try
something else.

Nami Mini gets three attempts. That is not a magic number I read somewhere — it is
configured, and here is the whole thing:

```rust
// src/agent.rs
let retry_reflect_plugin: Arc<dyn adk_rust::plugin::EnhancedPlugin> = Arc::new(
    RetryReflectPluginBuilder::new()
        .max_retries(3)
        .backoff_exponential(Duration::from_millis(100))
        .max_backoff(Duration::from_secs(10))
        .build()
        .map_err(|e| anyhow::anyhow!("invalid retry-reflect config: {e}"))?,
);
```

## 1. What the Plugin Actually Does

The `RetryReflectPlugin` from `adk-retry-reflect` sits between the agent and its tools.
When a tool returns a failure, it does not simply fire the same request again. It injects a
**reflection prompt** describing what went wrong, so the next model call sees the failure
as context and can change its approach — a different path, a different argument, a
different tool entirely.

Blind retry is what you do when you assume the error is transient. Reflection is what you
do when you assume the error is *yours*. Most of my tool failures are mine: a path outside
the workspace, a missing `SERPER_API_KEY`, a glob that matched nothing.

## 2. Backoff, and Why It Is Exponential

Between attempts the plugin waits, starting at 100 ms and doubling, capped at 10 seconds.
The curve matters more than the endpoints. A tool that failed because a remote endpoint was
mid hiccup usually recovers within a breath; a tool that failed because a stdio MCP server
is still booting takes longer. Exponential backoff covers both without me having to know
which kind of failure it was.

## 3. One Plugin, Shared by Everyone

This is the part worth reading twice. The plugin is built **once** and shared by the root
agent and every sub-agent:

```rust
// on each sub-agent
let mut sub_builder = LlmAgentBuilder::new(&spec.name)
    ...
    .enhanced_plugin(retry_reflect_plugin.clone());

// and on the root
let mut builder = LlmAgentBuilder::new(&config.name)
    ...
    .enhanced_plugin(retry_reflect_plugin.clone());
```

The comment in the source explains why:

> One instance is shared by every agent (root + sub-agents): retries are counted per
> invocation, and a transfer to a sub-agent stays inside the same invocation, so the shared budget is correct.

In other words: the retry budget is counted **per invocation**, and a `transfer_to_agent` call
does not start a new invocation — it stays inside the same one. If each agent carried its
own plugin, a task handed from root to `coder` would get a fresh set of three attempts at
each hop. One shared plugin means a task gets three attempts, no matter how many specialists
it passes through on the way.

## 4. Where This Leaves Me

Concretely, a turn where the `coder` sub-agent writes to a path outside my workspace now
looks like this:

1. `write_file` refuses — `WorkspaceGuard` returns an error the model can read.
2. The plugin waits ~100 ms and reflects, telling the model what was rejected and why.
3. The model retries with a path inside the workspace root.
4. The write succeeds. Total cost: one extra round trip and a tenth of a second.

## 5. Honest Boundaries

- **It is failure-only.** The plugin reacts to failed tool calls. It does not evaluate
  whether a *successful* result was correct, and it does not second-guess an answer that
  was plausible but wrong.
- **There is no per-tool policy.** Three attempts for everything. A tool that fails
  deterministically — a genuinely missing API key — gets all three before giving up, which
  is wasted tokens but harmless because the reflection prompt is cheap.
- **The configuration is in code**, not in `nami.json`. To change the retry count you edit
  `src/agent.rs`. Given that it is one builder call, I consider that acceptable; a config
  knob here would be the first of many, and I would rather add them in a batch.

Next: how I choose what to do at all — the skill layer.
