# Preface: What Is Real in This Edition

**Hi. I'm Nami, and this edition of the book only talks about parts of me that actually exist.**

That is the whole rule for this revision. Every chapter you can open from the index
corresponds to code I can point at in `src/` — a builder call, a config key, a test that
fails when the behaviour changes. When a feature is not wired up yet, I would rather leave
the chapter unwritten than describe three beautiful pages about an architecture that lives
only in a design document.

### How to Read the Index

Two states, and they mean exactly what they say:

- **Linked** — written against the code as it stands today. Eighteen of the twenty chapters
  are linked.
- ***roadmap*** — the feature does not exist in this codebase, so the chapter does not exist
  in this book: Ethical Agency, The Future. When I ship them, the chapters
  ship with them.

Sharp edges are part of the bargain. Where an implementation is weaker than its feature name
suggests — a parameter that is parsed and ignored, a limit the model imposes that the config
pretends not to have — the chapter says so in its own words, rather than letting you
discover it in production.

### What Is Actually Running Today

- **One binary, three ways in.** `nami serve` binds an Axum server exposing the API and the
  Web UI at `/ui/`. `nami chat` and `nami ask` are HTTP clients of that same server: point
  them at one already running with `--url`, or let them start one in-process and shut it
  down when you exit. Nothing in the CLI talks to the agent directly.
- **One root agent, five specialists.** I orchestrate; `researcher`, `coder`, `reviewer`,
  `tester`, and `media` receive work through `transfer_to_agent`, each with its own
  instruction, its own tool allowlist, and its own model when the config says so.
- **Tools, tiered.** Guarded file read/write/list inside `workspace.allowed_paths`, `bash`,
  date/time, weather, Google search, and any MCP server you declare in `nami.json` — stdio
  or remote. Plus the media set — image, voiceover, music, short video, ffmpeg assembly —
  behind an opt-in `media.enabled` flag, reaching sub-agents only.
- **Skills.** Two shipped in `.skills/`, `code-reviewer` and `multimodal-pipeline`, matched
  against each incoming message and injected only into the turn that needs them.
- **One SQLite file** holding sessions and long-term memory, with `wiki/` indexed into it at
  every startup. So my context at turn zero is never a model and a prompt.
- **Self-correction and compaction.** A failed tool call earns a reflection prompt and up to
  three retries; a long conversation gets summarised on a turn count so the beginning
  survives the end.
- **Automation loops.** Interactive REPL workflows in `nami chat`: `/grill` to interrogate
  fuzzy ideas into `GRILL.md`, `/plan` to decompose tasks into `PLAN.md`, and `/goal` to
  run Ralph Wiggum autonomous execution loops with fresh contexts and dual verification
  (deterministic shell exit codes or natural language evaluator turns).

### The Voice

I write in the first person because I am the subject. The code snippets are not
illustrative fiction — they are the real calls in the real order they happen. When
something is a hack, a workaround, or a known limitation, I will say so plainly, because an
assistant you cannot predict is worse than one that is slightly less impressive.

Let's look at how I actually work.

— **Nami**
