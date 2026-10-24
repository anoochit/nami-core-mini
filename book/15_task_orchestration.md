# Chapter 15: Task Orchestration

A single agent with a single prompt is a compromise that gets worse as the job broadens. Give
it a code-review persona and it starts reviewing things you asked it to write. Give it a
researcher persona and it starts citing sources in your commit messages. Separation of
concerns is not a software-only idea.

I am a **root orchestrator with hierarchical sub-agents**. There is no planner, no
graph, and no loop — one delegation step, chosen by the model.

## 1. The Team

From `nami.json`:

| Name | Description | Tools |
|------|-------------|-------|
| `researcher` | Deep web research and filesystem specialist | `search`, `devtools` |
| `coder` | Coding and workspace specialist | `devtools` |
| `reviewer` | Code review and security auditing specialist | `devtools` |
| `tester` | Testing and quality assurance specialist | `devtools` |

Four specialists, each with a `tools` allowlist (Chapter 9) and its own `instruction`. Root
keeps everything.

## 2. Registration

Each entry becomes a real `LlmAgent` attached as a sub-agent:

```rust
// src/agent.rs
for spec in config.enabled_agents() {
    let sub_model = match &spec.model {
        Some(override_model) => build_model_with(config, Some(override_model))?,
        None => model.clone(),
    };

    let mut sub_builder = LlmAgentBuilder::new(&spec.name)
        .description(spec.effective_description())
        .instruction(&spec.instruction)
        .model(sub_model)
        .enhanced_plugin(retry_reflect_plugin.clone());
    ...
    sub_agents.push(Arc::new(sub_builder.build()?) as Arc<dyn Agent>);
}
```

`LlmAgentBuilder::sub_agent()` attaches an `Arc<dyn Agent>`, and `adk-agent` does the rest:
it injects a `transfer_to_agent` tool whose enum is built from the registered sub-agents. I
never write a dispatcher. The dispatcher *is* the tool schema.

Note the sub-agents use `.instruction(...)` with plain text, while root uses
`.instruction_provider(...)`. That is safe here because specialist instructions in this repo
do not carry brace-heavy code samples — but if you add one that does, you will hit the
templating trap from Chapter 4 and should switch that agent to a provider too.

## 3. How Root Is Told to Delegate

Registration makes delegation *possible*. This makes it *likely* — appended to root's system
instruction at build time:

```rust
// src/agent.rs
fn root_delegation_guidance(config: &NamiConfig) -> Option<String> {
    let mut guide = String::from(
        "You are the main Nami assistant and orchestrator. You have specialist sub-agents \
         you can hand work to via the transfer_to_agent tool:\n",
    );
    for spec in std::iter::once(first).chain(agents) {
        guide.push_str(&format!("- `{}`: {}\n", spec.name, spec.effective_description()));
    }
    guide.push_str(
        "Handle simple conversational answers yourself. When a task clearly matches a \
         specialist, transfer to it and then summarize its result for the user.",
    );
    Some(guide)
}
```

Read that closing sentence as the actual policy, because it is:

1. **Answer simple things yourself.** Delegation is not free — it is another agent, another
   context, another round trip.
2. **Transfer only on a clear match.** Vague tasks stay with root.
3. **Summarise the result.** Root is the only agent talking to you; the specialist's raw
   output is intermediate.

And when there are no sub-agents, the function returns `None` — no guidance, no injected
transfer tool, no invitation to a party that is not happening.

## 4. The Shared Retry Budget

Sub-agents get the **same** retry-reflect plugin instance as root (Chapter 6), and the
reasoning is stated in the source:

> Retries are counted per invocation, and a transfer to a sub-agent stays inside the same
> invocation, so the shared budget is correct.

A transfer does not open a new invocation. Three attempts means three attempts for the whole
task, whether it stays with root or hops through three specialists. Per-agent plugins would
have quietly multiplied that budget by the depth of the delegation.

## 5. A Real Turn

```
you   > review the changes in src/tools/filesystem.rs

root  → transfer_to_agent("reviewer")
reviewer → read_file(...)          # devtools
reviewer → returns findings
root  → summarises findings for you
```

One transfer, one specialist, one summary back. What you see is root's answer; what the
reviewer saw was its own instruction and its own narrow toolset.

## 6. What I Do Not Have

- **No cross-agent messaging.** Specialists cannot talk to each other. The only edge is
  root → sub-agent.
- **No dynamic creation.** I cannot spawn a specialist that is not in `nami.json`.
- **No parallel fan-out.** One transfer at a time; there is no gather step.
- **No team-level state.** Each agent sees its own branch of history.

This is deliberately the shallowest orchestration that produces a real separation of
concerns. Chapter 16 shows how to reshape the team.

## 7. Testing It

The tests worth knowing are config-level, not behavioural:

- `test_root_delegation_guidance_lists_enabled_agents` — guidance lists enabled agents and
  omits disabled ones.
- `test_root_delegation_guidance_none_when_no_agents` — no agents means no guidance.

Both live in `src/agent.rs`. There is no test asserting a *particular* model chooses to
transfer, and there cannot be — that is the model's judgement, not my code's.

Next: how to reshape the team without touching Rust.
