# Chapter 7: Skill-Based Execution

A tool tells me *what I can do*. A skill tells me *how to do it properly*. Those are
different questions, and conflating them is why so many agents are capable but useless.

In my build the skill layer is one line:

```rust
// src/agent.rs
let mut builder = LlmAgentBuilder::new(&config.name)
    .description("Nami Assistant")
    .instruction_provider(...)
    .model(model)
    // Load skills from `.skills/` in the current working directory (adk-skill default)
    .with_auto_skills()?
```

One call, and everything else happens behind it.

## 1. Where Skills Come From

`with_auto_skills()` resolves to `with_skills_from_root(".")`, which means: *discover every
instruction file under `.skills/` in the directory I was started from*. Each directory
becomes one skill, each `SKILL.md` becomes one document in a `SkillIndex`.

This repository ships exactly one:

```
.skills/
└── code-reviewer/
    └── SKILL.md
```

The index is built **once, at agent construction time**. Add a skill and nothing happens
until I restart — the index is a struct on the agent, not a live directory watch.

## 2. How a Skill Reaches the Model

This is the part most descriptions get wrong, so here is the actual sequence.

When a turn starts, the agent assembles its preamble — global instruction, then instruction
— and pushes both into a `Vec<Content>` with role `system`:

```rust
// adk-agent/src/llm_agent.rs
let mut preamble = Vec::new();
...
if let Some(provider) = &self.instruction_provider {
    let instruction = provider(...).await?;
    if !instruction.is_empty() {
        preamble.push(Content::new("system").with_text(instruction));
    }
}
```

Then, and only then, the skill layer runs — against your message, not against the prompt:

```rust
let mut current_user_content = ctx.user_content().clone();
if let Some(index) = &self.skills_index {
    apply_skill_injection(&mut current_user_content, ...);
}
```

**The skill body is injected into the current user turn.** It never touches the system
instruction. Three consequences fall out of that, and they are all deliberate:

- **My stable instructions stay cacheable.** The system preamble is byte-identical from turn
  to turn, so providers that cache prompts get to cache it. Injecting variable skill text
  into it would invalidate that cache every time a different skill matched.
- **The skill arrives as fresh, immediate context.** A document appended to the message you
  just sent reads as "here is how to handle this", which is exactly the semantics I want.
- **No `{...}` templating.** System-instruction text goes through `inject_session_state`,
  which interprets braces as session variables. Skill bodies are full of literal code
  samples. On the user-turn path that hazard does not exist.

## 3. Matching Is Quiet

The index selects the best match for your message and injects at most one skill. If nothing
matches, nothing is injected and the turn looks exactly as it would with no skills installed.
There is no "no skill found" chatter in my replies — you should not have to care whether the
mechanism fired.

## 4. Everyone Gets the Same Index

Root is not the only agent with skills. The sub-agent loop attaches the loader too:

```rust
// src/agent.rs
// Sub-agents also load skills from `.skills/` — adk-skill injects only the skill
// that matches the task at hand (e.g. `multimodal-pipeline` when producing media)
.with_auto_skills()?
```

So every agent — root and all four specialists — reads the same `.skills/` directory at
build time and matches against the same index. Selection still happens per agent, against
that agent's own user turn, which is what keeps this useful rather than noisy: the `media`
specialist picks up `multimodal-pipeline` when it is handed a video job, and the `reviewer`
picks up `code-reviewer` when it is handed a review. Each gets the discipline relevant to
the task it received, not a bundle of every procedure in the repository.

Two practical consequences:

- **Adding a skill helps every agent at once.** There is no per-agent registration step.
- **A skill is scoped by task, not by agent.** A specialist does not "own" a skill; it
  receives it when the incoming request matches. Give `coder` a task that reads like a
  review and it may well get the review skill too.

## 5. Skills vs. the Alternatives

| Mechanism | Reaches the model as | Use it for |
|-----------|---------------------|------------|
| Tool | JSON schema + runtime result | A capability: run this, fetch that, write this file |
| Skill | Procedural text in the user turn | A discipline: the order, the pitfalls, the house style |
| Sub-agent | A transfer target with its own context | A boundary: separate context window, separate tool allowlist, separate model |

A skill never executes anything. If your `SKILL.md` says "run the tests before answering",
it is still the model's job to call the tool. Skills raise the quality of the calls I make;
they do not make the calls for me.

Next: how to write one that actually gets used.
