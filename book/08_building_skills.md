# Chapter 8: Building Skills

Here is the good news about skills: writing one is editing a Markdown file. Here is the bad
news: the hard part was never the file format, it is writing a `description` good enough to
make the skill fire when it should.

## 1. The Minimum Viable Skill

```
.skills/
└── my-skill/
    └── SKILL.md
```

```markdown
---
name: my-skill
description: What this skill does and when an agent should reach for it.
tags: [rust, testing]
---

# My Skill

1. Do this first.
2. Then this.
```

Both frontmatter fields are required. `name` must be 1–64 characters of lowercase letters,
digits, and hyphens. `description` is the sentence the selection layer matches against. A
file that fails validation is **skipped without an error** — a malformed skill fails
silently, which is the single most common "why isn't my skill working" cause.

## 2. The Full Frontmatter

Everything below `name` and `description` is optional:

| Field | Type | Purpose |
|-------|------|---------|
| `version` | string | Identifier, e.g. `1.0.0` |
| `license` | string | SPDX id or bundled licence file |
| `compatibility` | string | Environment needs, e.g. "Requires network access" |
| `tags` | list | Categorisation and filtering |
| `allowed-tools` | list or delimited string | Pre-approved tools (experimental) |
| `references` | list | Supporting files, e.g. `references/checklist.md` |
| `trigger` | bool | `true` = only when explicitly invoked by name |
| `hint` | string | Guided-input hint for slash commands |
| `triggers` | list of globs | File patterns that activate the skill, e.g. `["*.rs"]` |
| `metadata` | map | Arbitrary extension data |

`trigger: true` is worth knowing about. It takes the skill out of automatic matching
entirely, so it only loads when you name it. Use it for procedures you want on demand and
never by accident.

## 3. The Body Is a Procedure, Not an Essay

The body is what gets injected into the user turn when the skill matches. That is a scarce
resource — it rides on every matching turn. So write it as instructions the model can
execute, not as background reading:

```markdown
# Rust Code Review

When reviewing Rust code in this workspace:

1. Check error handling: `Result` and `anyhow`/`thiserror` used appropriately.
2. Check memory safety: avoid `unsafe` blocks unless documented and justified.
3. Check concurrency: `tokio` primitives and `Arc<RwLock<T>>` for shared state.
4. Check style: Rust Edition 2024, clippy zero-warning policy.
```

That is the real `.skills/code-reviewer/SKILL.md` from this repository, and it is a good
template: numbered steps, concrete names, no preamble. Every line of prose you add is a line
the model has to weigh against your actual request.

## 4. Where to Put Knowledge Instead

Before making a skill, check that it is the right container:

- **Stable fact about this project** (architecture, conventions, decisions) → `wiki/`. It
  gets indexed into long-term memory at startup and is always available. Chapter 5.
- **Personality, role, output rules** → `system_instruction` in `nami.json`. Always in
  context, cacheable. Chapter 4.
- **A repeatable procedure with pitfalls** → a skill. This chapter.
- **A capability I lack** → a tool in `src/tools/`. Chapter 10.

Skills are for the middle case: procedure that applies to *some* messages and would be noise
on the rest.

## 5. Making It Fire

The `description` is the trigger. Write it the way you would write a routing rule, naming
the situations and the vocabulary that should activate it:

| Weak description | Strong description |
|------------------|--------------------|
| "Helps with code review" | "Use when reviewing Rust code for error handling, unsafe blocks, and concurrency" |
| "Deployment stuff" | "Use when deploying to production, rotating keys, or diagnosing a failed release" |

Test it the honest way: restart `nami chat`, ask the kind of question the skill should
cover, and look at the `tool` / `answer` sections of the streamed output. If the skill is not
shaping the answer, the description is not matching — the body is not the problem.

## 6. The Part I Do Not Have Yet

I load skills. I do not write them.

There is no mechanism here by which I synthesise a new `SKILL.md` from what I learned during
a task and register it for the next run. When I discover a procedure that works, I write it
to `wiki/` as a lesson, and you — or a later version of me with a skill-writer tool — decide
whether it deserves to become a skill.

That is the honest boundary of this chapter. Everything above it is running code.

Next: the four layers that decide what I can actually reach for.
