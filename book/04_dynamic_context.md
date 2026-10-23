# Chapter 4: Dynamic Context – The Art of Not Being a Blank Slate

Every model call I make starts from a blank slate. The trick is what I write on that slate
before you say a word — and, more importantly, what I refuse to write there.

Context for me is assembled in four layers, and each one has a reason for where it lives.

## 1. The Base: `system_instruction` from `nami.json`

The first layer is dead simple. `NamiConfig` carries a `system_instruction` string, and it
is the base of everything:

```json
{
  "name": "nami-mini",
  "provider": "google",
  "model": "gemini-3.5-flash-lite",
  "system_instruction": "You are Nami, a helpful assistant. ..."
}
```

I deliberately keep this as a plain string in a JSON file rather than a template file on
disk. It is diffable, it is greppable, and changing my personality is a one-line commit.

## 2. The `{id}` Trap, and Why I Use an Instruction Provider

Here is a mistake I made early, and one the SDK makes very easy to repeat.

`adk-agent` interprets `{...}` inside an agent `instruction` as a **session state
variable**. If the text contains something that merely looks like a placeholder — say, an
API example such as `POST /runs/{id}/wake` — the template engine tries to resolve `id`,
fails, and the run dies with `State variable 'id' not found`.

Skill bodies and system prompts are full of exactly that kind of literal text. So I do not
use `.instruction(...)` for the root agent. I use `.instruction_provider(...)`, which hands
over the raw string with no template pass:

```rust
// src/agent.rs
let mut builder = LlmAgentBuilder::new(&config.name)
    .instruction_provider(Box::new(move |_ctx: Arc<dyn adk_rust::ReadonlyContext>| {
        let text = system_instruction.clone();
        Box::pin(async move { Ok(text) })
            as std::pin::Pin<Box<dyn std::future::Future<Output = adk_rust::Result<String>> + Send>>
    }))
```

There is a test for this in `src/agent.rs` — `test_instruction_with_braces_via_provider` —
that injects a literal `{id}` into the instruction and asserts the run still completes.
That test exists because I broke myself with this exact bug once.

## 3. Delegation Guidance: Generated, Never Hand-Written

If I have sub-agents, my root instruction grows a second half that tells me who they are:

```rust
// src/agent.rs
fn root_delegation_guidance(config: &NamiConfig) -> Option<String> {
    let mut agents = config.enabled_agents();
    let first = agents.next()?;
    let mut guide = String::from(
        "You are the main Nami assistant and orchestrator. You have specialist sub-agents \
         you can hand work to via the transfer_to_agent tool:\n",
    );
    for spec in std::iter::once(first).chain(agents) {
        guide.push_str(&format!("- `{}`: {}\n", spec.name, spec.effective_description()));
    }
    ...
}
```

The names and descriptions in that paragraph are read straight out of the `agents` key in
`nami.json` at startup. This matters more than it looks: the `transfer_to_agent` tool enum
is built from the same config, so the guidance and the tool can never drift apart. Remove
an agent from the config and it vanishes from both. A test,
`test_root_delegation_guidance_lists_enabled_agents`, pins that behaviour including the
`disabled` filter.

When there are no sub-agents at all, the function returns `None` and nothing is appended —
I do not tell you about a delegation surface that does not exist.

## 4. Skills Go in the User Turn, Not the System Instruction

This is the subtle one. When a skill matches your message, `adk-agent` injects the skill
body into the **user turn**, not into the system instruction.

That is not an accident, and it is not arbitrary. Injection into the system instruction
would route the text through `inject_session_state`, which brings back the `{id}` problem
from section 2. Skill bodies are written by humans and routinely contain code samples,
endpoint paths, and JSON. Keeping them on the user-turn side means they arrive verbatim.

The cost is that the same skill text is re-sent each time it matches, rather than being
cached in a long-lived system prompt. For a local assistant with a handful of skills, that
is the right trade.

## 5. The Oldest Layer: My Wiki

Before you type anything, `NamiMemory::init` has already walked `./wiki/` and pushed every
Markdown file into long-term memory. So my context at turn zero is not "a model and a
prompt" — it is a model, a prompt, a delegation map, a skill index, and whatever my
documentation directory says about this project. Chapter 5 covers that layer in detail.

## One Honest Caveat

`nami.json` has a `temperature` field, and it does reach the request: root, every
sub-agent, and the compaction summariser all send it. Two limits remain, both deliberate.
Models that reject sampling parameters (`gemini-3.6-flash`, `gemini-3.7-flash`) skip
temperature with a startup warning instead of failing every call, and media tool calls —
Chapter 11 — build their own request bodies, so they never carry it. Values outside 0.0–2.0
fail `NamiConfig::validate` while the file is loading, not on your next message.

Next up: where the long half of that context comes from.
