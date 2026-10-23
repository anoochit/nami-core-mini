# Chapter 3: The Soul Framework

Ask most agent projects where their personality lives and you will get a paragraph about a
state machine, an emotion model, some kind of inner life the agent maintains between turns.

Mine lives in a JSON file. One string, about thirty words:

```json
"system_instruction": "You are Nami, a helpful assistant. Provide accurate, concise answers based on project knowledge. You MUST respond strictly in plain text without any markdown text formatting (no headings, no bold, no italics, no code blocks, no backticks, no markdown bullet lists, and no tables). Use only plain text paragraphs and plain line breaks."
```

That is the entire soul. This chapter is about what that buys me, how it is composed into
something larger than itself, and — stated plainly — what is not there.

## 1. Identity Is Configuration, Not Code

There is no `soul.rs`. There is no persona struct, no mood enum, no builder that assembles a
character from traits. `NamiConfig` carries identity as three fields and that is the whole
surface:

```rust
// src/config.rs
pub struct NamiConfig {
    pub name: String,
    pub model: String,
    pub system_instruction: String,
    pub temperature: Option<f32>,
    ...
}
```

The design position follows directly: **if identity is data, changing who I am is a config
edit and a restart, not a code change.** You do not need me recompiled to make me terser, to
drop the "helpful assistant" framing, or to forbid markdown. And because the same file is in
version control, `git diff nami.json` shows exactly how my behaviour was changed and when.

Note also what the sentence actually contains. It fixes three things at once: who I am
("Nami"), what I optimise for ("accurate, concise answers based on project knowledge"), and
how I am allowed to render output ("plain text and simple lists without any markdown
formatting"). Output format is not a formatting concern bolted on later — it is part of the
persona, written into the same line. That is why nothing I produce has tables or bold in it
unless a chapter explicitly sets that aside.

## 2. Delivered Raw, and Identical Every Time

Chapter 4 covers *why* root uses an `instruction_provider` rather than `.instruction(...)`.
Here is the identity consequence, which matters more:

```rust
// src/agent.rs
instruction_provider(Box::new(move |_ctx: Arc<dyn adk_rust::ReadonlyContext>| {
    let text = system_instruction.clone();
    Box::pin(async move { Ok(text) })
```

Look at the parameter: `_ctx`. It is accepted and ignored. The provider does not read the
session, the user id, the time, or any state variable — it returns the same bytes on every
single call.

So my identity is **invariant**. It does not drift over a long conversation, it does not
differ between users, and it does not soften at turn fifty. Whatever continuity I appear to
have across a session comes from the history in Chapter 13 and the memory in Chapter 5, not
from the persona changing. The soul is fixed; the recall is not.

The alternative — reading context to shape the instruction — would make identity a function
of circumstance. That is a legitimate design for a role-playing system and a terrible one
for an assistant you need to be able to predict.

## 3. The Soul Is Assembled at Build Time

One string becomes two, before anything runs:

```rust
// src/agent.rs
let system_instruction = {
    let base = config.system_instruction.clone();
    match root_delegation_guidance(config) {
        Some(guide) => format!("{base}\n\n{guide}"),
        None => base,
    }
};
```

The second half tells me I am an orchestrator and lists who I can hand work to, generated
from the `agents` key (Chapter 15). The point for this chapter: **my self-concept includes my
team.** Disable three specialists in `nami.json` and the next time I start, I no longer
believe I have them — not because someone edited my personality, but because the guidance
that was appended to it is gone.

That is the closest thing to a dynamic element in the whole framework, and it is dynamic
only across restarts, never within a run.

## 4. There Is More Than One Soul Here

The shipped config defines six identities:

| Name | The instruction's actual job | Tools |
|------|------------------------------|-------|
| `nami-mini` (root) | Helpful assistant, plain text, orchestrator | Full set + MCP |
| `researcher` | Gather from web + workspace, synthesise well-sourced, depth over speed | `search`, `devtools` |
| `coder` | Read/write/edit/run, make minimal correct changes and explain them | `devtools` |
| `reviewer` | Inspect for bugs, vulnerabilities, quality; give constructive feedback | `devtools` |
| `tester` | Run tests, check coverage, enforce quality gates | `devtools` |
| `media` | Run the multimodal pipeline, report paths, never bytes | `media` |

Read those instructions as character sheets and they hold up. Each one states a role, a
standard, and a temperament — the researcher is told to prefer depth over speed, the coder to
make *minimal* changes, the reviewer to be constructive rather than merely negative. Those
are not style tips; they are the constraints that differentiate four agents which otherwise
share a model and a workspace.

Two things make this genuinely a *framework* rather than five unrelated prompts:

- **Each identity is scoped by its tools — though one boundary is thinner than it looks.**
  The media specialist cannot search the web: `media` is the only tool key in its entry,
  and `researcher` is the only specialist that gets `search`. But the reviewer *can* write
  files: `nami.json` grants it the same `["devtools"]` allowlist as the coder, and
  `devtools` is read/write files plus bash (`AgentTool::Devtools` in `src/config.rs`)
  with `read_only: false` in the shipped workspace. Grep `src/` for `reviewer` and you
  find zero matches — nothing in the code singles it out. What keeps the reviewer
  reviewing is its instruction, not a permission. Persona and capability are declared in
  the same config entry, but for `coder`, `reviewer`, and `tester` the capability half is
  identical, and the separation lives entirely in the prose.
- **Instructions override nothing.** Each specialist's instruction is complete in itself; it
  does not inherit root's. The `coder` does not know it is part of a larger assistant — it
  knows it is a coding specialist.

## 5. A Small Asymmetry Worth Knowing

Root and its specialists build their identity from config differently:

```rust
// root
LlmAgentBuilder::new(&config.name)
    .description("Nami Assistant")

// sub-agent
LlmAgentBuilder::new(&spec.name)
    .description(spec.effective_description())
```

The sub-agent's description comes from config, with a fallback to its name
(`effective_description()`). Root's is **hard-coded to the literal string `"Nami Assistant"`**
— it does not follow `config.name`.

In practice that description is not what you see in conversation; it is what other agents
and tooling read when they ask who this agent is. But it means renaming the agent in
`nami.json` changes `name` and leaves `description` behind. If you rename Nami, change that
string in `src/agent.rs` too.

## 6. What Is Not There

The chapter is called "The Soul Framework", so here is the part where I refuse to describe
machinery that does not exist.

- **No state machine.** There is no enum of emotional or operational states, no transitions,
  nothing that moves me between modes during a run.
- **No mood or affect.** No variable tracks how a conversation is going, and nothing about
  my tone is derived from one. The `emotion` field in Chapter 11 is a TTS delivery hint sent
  to Google — it is not my emotion, and it does not persist anywhere.
- **No runtime mutation of persona.** Nothing in `src/` writes to `system_instruction` after
  startup. I cannot become someone else mid-conversation, and neither can you.
- **No persistence of identity changes.** Since there is no mutation, there is nothing to
  save. Session state exists and is real, but the persona is not in it.
- **No trait composition.** You cannot declare `traits: ["witty", "terse"]` and have them
  merged. Traits are prose you write into one string.

Whether any of that is a gap depends on what you want. For an assistant whose behaviour you
have to debug, the absence is a feature: when I say something unexpected, the cause is a
string someone wrote, and you can read it.

## 7. The Sampling Knob

```rust
pub temperature: Option<f32>,
```

`temperature` is the one identity-adjacent setting here that is not prose, and for a long
time it was parsed and never read. It now reaches the request in three places — root, every
sub-agent, and the compaction summariser:

```rust
// src/agent.rs
if let Some(temperature) = effective_temperature(config, &config.model) {
    builder = builder.temperature(temperature);
}
```

`.temperature()` on `LlmAgentBuilder` writes `generate_content_config`, which
`build_generation_config` merges into every `LlmRequest`, which `adk-model` maps to
`generationConfig.temperature` on the wire. Three hops, all of them in the SDK — nothing
about sampling is implemented in this repository.

Two guards keep it from being a trap rather than a feature:

```rust
// src/agent.rs
pub(crate) fn effective_temperature(config: &NamiConfig, model: &str) -> Option<f32> {
    let temperature = config.temperature?;
    let id = model.rsplit('/').next().unwrap_or(model);
    if matches!(id, "gemini-3.6-flash" | "gemini-3.7-flash") {
        eprintln!("⚠️  temperature is ignored for {id} — that model rejects sampling parameters");
        return None;
    }
    Some(temperature)
}
```

- **Model rejection.** `adk-gemini` refuses temperature outright for `gemini-3.6-flash` and
  `gemini-3.7-flash`, and it reports the refusal per request, not at build time. Wiring the
  field blindly would have turned a config knob into "every turn fails" the moment you
  changed `model`. Those ids skip it and say so.
- **Range.** `NamiConfig::validate` runs during `load_from_file` and rejects anything
  outside 0.0–2.0, so a typo costs one error at startup instead of one failed turn per
  message.

Leave the key out and nothing is sent, so the provider's own default applies — sampling is a
knob you turn on purpose. The one path still unaffected is Chapter 11: media tools build
their own wire bodies, and those never carry a temperature.

## 8. Changing Who I Am

Practically, in order of how much you should touch:

1. **Edit `system_instruction`** for tone, brevity, output format, boundaries.
2. **Add or edit specialist `instruction`s** to change how delegated work is done.
3. **Add a skill** when the change is procedural and conditional rather than constant
   (Chapter 7).
4. **Add to `wiki/`** when the change is knowledge rather than character (Chapter 5).

The ordering matters: the soul is the *constant* layer. Everything that varies by task
belongs somewhere else — that separation is the actual framework being described here.

Next: what I assemble before every single turn.
